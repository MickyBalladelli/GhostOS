#![no_std]
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native;

use ghostos_script::{
    CowSandbox, LogicalNameControl, SandboxCommandHandler, SandboxDecision, SandboxExecutor,
    SandboxReceipt, Script, ScriptContext, ScriptEngine, ScriptEvent, ToolSchemaExporter,
};
use ghostos_shell::parser::{CommandRegistration, CommandRegistry};
use ghostos_auth::{
    CapabilityCaveat, CapabilityKey, CapabilityLease, CryptographicCapability, LeaseContext,
    LeaseError, TokenError, TransportRights,
};
use ghostos_fabric::NodeId;
use ghostos_kernel::Rights;
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::SynFs;
use ghostos_system_model::command::StructuredOutput;

pub const DEFAULT_AGENT_GRANT_CAPACITY: usize = 64;
const AGENT_TENANT: u64 = 1;
const AGENT_PURPOSE: u64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    /// `RUN /SANDBOX`: execute against a private CoW root and always discard it.
    Sandbox,
    /// Approved `RUN`: publish a successful private root as one atomic commit.
    Commit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTaskScope {
    pub resource: u64,
    pub rights: Rights,
    pub transports: TransportRights,
    pub max_lifetime_us: u64,
}

impl AgentTaskScope {
    pub fn new(
        resource: u64,
        rights: Rights,
        transports: TransportRights,
        max_lifetime_us: u64,
    ) -> Result<Self, Error> {
        crate::native::scope(resource, rights.bits(), transports.bits(), max_lifetime_us)
            .map_err(|_| Error::InvalidScope)?;
        Ok(Self {
            resource,
            rights,
            transports,
            max_lifetime_us,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AgentGrant {
    token: CryptographicCapability,
    lease: CapabilityLease,
    subject: NodeId,
    resource: u64,
    rights: Rights,
    transports: TransportRights,
    expires_at_us: u64,
}

/// Fixed-capacity issuer and replay ledger for agent task capabilities.
///
/// Tokens are newly signed from a broader parent capability, restricted to one
/// task scope and one agent, and accepted exactly once through `consume`.
pub struct SingleUseCapabilityIssuer<const GRANTS: usize = DEFAULT_AGENT_GRANT_CAPACITY> {
    issuer: NodeId,
    key: CapabilityKey,
    grants: [Option<AgentGrant>; GRANTS],
    revocation_epoch: u64,
    next_nonce: u64,
}

impl<const GRANTS: usize> SingleUseCapabilityIssuer<GRANTS> {
    pub const fn new(issuer: NodeId, key: CapabilityKey, boot_nonce: u64) -> Self {
        Self {
            issuer,
            key,
            grants: [None; GRANTS],
            revocation_epoch: 1,
            next_nonce: boot_nonce,
        }
    }

    pub fn mint(
        &mut self,
        parent: &CryptographicCapability,
        subject: NodeId,
        scope: AgentTaskScope,
        lifetime_us: u64,
        now_us: u64,
    ) -> Result<CryptographicCapability, Error> {
        crate::native::parent(
            parent.issuer.raw(),
            self.issuer.raw(),
            parent.resource,
            scope.resource,
            parent.revocation_epoch,
            self.revocation_epoch,
        )
        .map_err(|_| Error::ScopeViolation)?;
        crate::native::covers(parent.effective_rights().bits(), scope.rights.bits())
            .map_err(|_| Error::ScopeViolation)?;
        crate::native::transport_lifetime(
            parent.effective_transports().bits(),
            scope.transports.bits(),
            lifetime_us,
            scope.max_lifetime_us,
        )
        .map_err(|_| Error::ScopeViolation)?;
        parent
            .verify(
                self.key,
                parent.effective_subject(),
                scope.rights,
                scope.transports,
                now_us,
                self.revocation_epoch,
            )
            .map_err(Error::Token)?;

        let expires_at_us = crate::native::add_time(now_us, lifetime_us)
            .map_err(|_| Error::ScopeViolation)?;
        if !crate::native::within_expiry(expires_at_us, parent.effective_expiry()) {
            return Err(Error::ScopeViolation);
        }

        let views = self.grant_views();
        let grant_slot = crate::native::slot(&views, now_us).map_err(|_| Error::Capacity)?;
        self.next_nonce = crate::native::next_id(self.next_nonce);
        let token = CryptographicCapability::issue(
            self.key,
            self.issuer,
            subject,
            scope.resource,
            scope.rights,
            scope.transports,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            self.next_nonce,
        )
        .map_err(Error::Token)?
        .attenuate(CapabilityCaveat {
            subject: Some(subject),
            rights: scope.rights,
            transports: scope.transports,
            expires_at_us,
        })
        .map_err(Error::Token)?;
        let lease = CapabilityLease::issue(
            self.key,
            self.issuer,
            subject,
            self.issuer,
            scope.resource,
            AGENT_TENANT,
            1,
            AGENT_PURPOSE,
            scope.rights,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            token.nonce,
        )
        .map_err(map_lease_error)?;
        self.grants[grant_slot] = Some(AgentGrant {
            token,
            lease,
            subject,
            resource: scope.resource,
            rights: scope.rights,
            transports: scope.transports,
            expires_at_us,
        });
        Ok(token)
    }

    /// Verify and consume a task token. Removal happens only after all checks
    /// pass, so a failed authorization does not burn a valid grant.
    pub fn consume(
        &mut self,
        token: &CryptographicCapability,
        subject: NodeId,
        required: Rights,
        transport: TransportRights,
        now_us: u64,
    ) -> Result<(), Error> {
        crate::native::required(required.bits()).map_err(|_| Error::InvalidScope)?;
        let slot = self
            .grants
            .iter()
            .position(|entry| {
                entry.is_some_and(|grant| {
                    grant.token == *token
                        && grant.subject == subject
                        && grant.resource == token.resource
                })
            })
            .ok_or(Error::AlreadyConsumed)?;
        let grant = self.grants[slot].ok_or(Error::AlreadyConsumed)?;
        crate::native::grant_access(
            now_us,
            grant.expires_at_us,
            grant.rights.bits(),
            required.bits(),
            grant.transports.bits(),
            transport.bits(),
        )
        .map_err(|_| Error::AccessDenied)?;
        token
            .verify(
                self.key,
                subject,
                required,
                transport,
                now_us,
                self.revocation_epoch,
            )
            .map_err(Error::Token)?;
        grant
            .lease
            .authorize(
                self.key,
                LeaseContext {
                    subject,
                    audience: self.issuer,
                    object: token.resource,
                    tenant: AGENT_TENANT,
                    generation: 1,
                    purpose: AGENT_PURPOSE,
                    required,
                    now_us,
                },
                self.revocation_epoch,
            )
            .map_err(map_lease_error)?;
        self.grants[slot] = None;
        Ok(())
    }

    pub fn revoke_all(&mut self) {
        self.revocation_epoch = crate::native::next_id(self.revocation_epoch);
        self.grants.fill(None)
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn active_grants(&self, now_us: u64) -> usize {
        crate::native::active(&self.grant_views(), now_us)
    }

    fn grant_views(&self) -> [crate::native::Grant; GRANTS] {
        self.grants.map(|grant| match grant {
            Some(grant) => crate::native::Grant {
                expires_at_us: grant.expires_at_us,
                occupied: true,
            },
            None => crate::native::Grant {
                expires_at_us: 0,
                occupied: false,
            },
        })
    }
}

fn map_lease_error(error: LeaseError) -> Error {
    match error {
        LeaseError::Invalid => Error::Token(TokenError::Invalid),
        LeaseError::InvalidSignature => Error::Token(TokenError::InvalidSignature),
        LeaseError::Expired
        | LeaseError::NotYetValid
        | LeaseError::Revoked
        | LeaseError::Replay
        | LeaseError::ReplayCapacity
        | LeaseError::SubjectMismatch
        | LeaseError::AudienceMismatch
        | LeaseError::ObjectMismatch
        | LeaseError::TenantMismatch
        | LeaseError::GenerationMismatch
        | LeaseError::PurposeMismatch
        | LeaseError::RightsDenied => Error::AccessDenied,
    }
}

pub struct RunRequest<'a> {
    pub source: &'a str,
    pub mode: RunMode,
    pub authority: &'a CryptographicCapability,
    pub subject: NodeId,
    pub task_rights: Rights,
    pub transport: TransportRights,
    pub now_us: u64,
}

pub struct RunOutcome {
    pub status: Status,
    pub output: Option<StructuredOutput>,
    pub receipt: SandboxReceipt,
}

impl RunOutcome {
    pub const fn committed(&self) -> bool {
        self.receipt.committed
    }
}

/// Completed script whose CoW root is still private.
///
/// Callers may inspect staged files and typed output before approving the exact
/// transaction. A dropped prepared run is rolled back by `CowSandbox`.
pub struct PreparedRun<'filesystem, const MAX_BLOCKS: usize> {
    status: Status,
    output: Option<StructuredOutput>,
    sandbox: CowSandbox<'filesystem, MAX_BLOCKS>,
    commit_authorized: bool,
}

impl<const MAX_BLOCKS: usize> PreparedRun<'_, MAX_BLOCKS> {
    pub const fn status(&self) -> Status {
        self.status
    }

    pub const fn output(&self) -> Option<&StructuredOutput> {
        self.output.as_ref()
    }

    pub const fn can_commit(&self) -> bool {
        self.commit_authorized && self.status.is_success()
    }

    pub const fn base_generation(&self) -> u64 {
        self.sandbox.base_generation()
    }

    pub fn staged_generation(&self) -> u64 {
        self.sandbox.staged_generation()
    }

    pub fn operations(&self) -> u32 {
        self.sandbox.operations()
    }

    pub fn lookup(&self, path: &str) -> Result<ghostos_ghostfs::FileVersion, Error> {
        self.sandbox.lookup(path).map_err(Error::Script)
    }

    pub fn read(
        &self,
        path: &str,
        destination: &mut [u8],
    ) -> Result<ghostos_ghostfs::ReadResult, Error> {
        self.sandbox.read(path, destination).map_err(Error::Script)
    }

    pub fn finish(self, mode: RunMode) -> Result<RunOutcome, Error> {
        if mode == RunMode::Commit && !self.commit_authorized {
            return Err(Error::CommitNotAuthorized);
        }
        let commit = crate::native::finish(
            mode == RunMode::Commit,
            self.commit_authorized,
            self.status.is_success(),
        )
        .map_err(|_| Error::CommitNotAuthorized)?;
        let decision = if commit {
            SandboxDecision::Commit
        } else {
            SandboxDecision::Discard
        };
        let receipt = self.sandbox.finish(decision).map_err(Error::Script)?;
        Ok(RunOutcome {
            status: self.status,
            output: self.output,
            receipt,
        })
    }

    pub fn approve(self) -> Result<RunOutcome, Error> {
        self.finish(RunMode::Commit)
    }

    pub fn discard(self) -> Result<RunOutcome, Error> {
        self.finish(RunMode::Sandbox)
    }
}

/// Native boundary exposed to an AI agent runtime.
///
/// It reflects live command registrations, mints narrow one-use authority, and
/// runs procedures entirely through a GhostFS transaction.
pub struct AgentBridge<const GRANTS: usize = DEFAULT_AGENT_GRANT_CAPACITY> {
    capabilities: SingleUseCapabilityIssuer<GRANTS>,
}

impl<const GRANTS: usize> AgentBridge<GRANTS> {
    pub const fn new(issuer: NodeId, key: CapabilityKey, boot_nonce: u64) -> Self {
        Self {
            capabilities: SingleUseCapabilityIssuer::new(issuer, key, boot_nonce),
        }
    }

    pub fn tool_schemas<const COMMANDS: usize>(
        &self,
        registry: &CommandRegistry<COMMANDS>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        ToolSchemaExporter::export(registry, destination).map_err(Error::Script)
    }

    pub fn resolve_tool<const COMMANDS: usize>(
        &self,
        registry: &CommandRegistry<COMMANDS>,
        function_name: &str,
    ) -> Result<Option<CommandRegistration>, Error> {
        ToolSchemaExporter::resolve(registry, function_name).map_err(Error::Script)
    }

    pub fn mint_task_capability(
        &mut self,
        parent: &CryptographicCapability,
        subject: NodeId,
        scope: AgentTaskScope,
        lifetime_us: u64,
        now_us: u64,
    ) -> Result<CryptographicCapability, Error> {
        self.capabilities
            .mint(parent, subject, scope, lifetime_us, now_us)
    }

    pub fn run<
        const COMMANDS: usize,
        const STATEMENTS: usize,
        const SYMBOLS: usize,
        const MAX_BLOCKS: usize,
        Handler,
        LogicalNames,
    >(
        &mut self,
        request: RunRequest<'_>,
        registry: &CommandRegistry<COMMANDS>,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        handler: &mut Handler,
        context: &mut ScriptContext<LogicalNames, SYMBOLS>,
    ) -> Result<RunOutcome, Error>
    where
        Handler: SandboxCommandHandler<MAX_BLOCKS>,
        LogicalNames: LogicalNameControl,
    {
        let mode = request.mode;
        self.prepare::<COMMANDS, STATEMENTS, SYMBOLS, MAX_BLOCKS, Handler, LogicalNames>(
            request, registry, filesystem, handler, context,
        )?
        .finish(mode)
    }

    pub fn prepare<
        'filesystem,
        const COMMANDS: usize,
        const STATEMENTS: usize,
        const SYMBOLS: usize,
        const MAX_BLOCKS: usize,
        Handler,
        LogicalNames,
    >(
        &mut self,
        request: RunRequest<'_>,
        registry: &CommandRegistry<COMMANDS>,
        filesystem: &'filesystem mut SynFs<MAX_BLOCKS>,
        handler: &mut Handler,
        context: &mut ScriptContext<LogicalNames, SYMBOLS>,
    ) -> Result<PreparedRun<'filesystem, MAX_BLOCKS>, Error>
    where
        Handler: SandboxCommandHandler<MAX_BLOCKS>,
        LogicalNames: LogicalNameControl,
    {
        let script = Script::<STATEMENTS>::compile(request.source).map_err(Error::Script)?;
        let required = Rights::from_bits(crate::native::run_rights(
            request.task_rights.bits(),
            request.mode == RunMode::Commit,
        ))
        .expect("run rights stay inside the rights mask");
        self.capabilities.consume(
            request.authority,
            request.subject,
            required,
            request.transport,
            request.now_us,
        )?;

        let mut sandbox = CowSandbox::new(filesystem);
        let (status, output) = {
            let adapter = BorrowedHandler { handler };
            let mut executor = SandboxExecutor::new(&mut sandbox, adapter);
            let mut engine = ScriptEngine::<STATEMENTS>::new();
            engine.start(script).map_err(Error::Script)?;
            let mut output = None;
            let status = loop {
                match engine
                    .poll(registry, &mut executor, context)
                    .map_err(Error::Script)?
                {
                    ScriptEvent::Pending | ScriptEvent::StatementFailed(_) => {}
                    ScriptEvent::PipelineComplete(value) => output = Some(value),
                    ScriptEvent::Complete(status) | ScriptEvent::Failed(status) => break status,
                    ScriptEvent::Cancelled => return Err(Error::Cancelled),
                }
            };
            (status, output)
        };
        Ok(PreparedRun {
            status,
            output,
            sandbox,
            commit_authorized: request.mode == RunMode::Commit,
        })
    }

    pub const fn capabilities(&self) -> &SingleUseCapabilityIssuer<GRANTS> {
        &self.capabilities
    }

    pub fn capabilities_mut(&mut self) -> &mut SingleUseCapabilityIssuer<GRANTS> {
        &mut self.capabilities
    }
}

struct BorrowedHandler<'a, Handler> {
    handler: &'a mut Handler,
}

impl<Handler, const MAX_BLOCKS: usize> SandboxCommandHandler<MAX_BLOCKS>
    for BorrowedHandler<'_, Handler>
where
    Handler: SandboxCommandHandler<MAX_BLOCKS>,
{
    fn execute(
        &mut self,
        sandbox: &mut CowSandbox<'_, MAX_BLOCKS>,
        command: ghostos_shell::parser::CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<StructuredOutput, Status> {
        self.handler.execute(sandbox, command, pipeline_input)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AccessDenied,
    AlreadyConsumed,
    Cancelled,
    Capacity,
    CommitNotAuthorized,
    InvalidScope,
    ScopeViolation,
    Script(ghostos_script::Error),
    Token(TokenError),
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied
            | Self::AlreadyConsumed
            | Self::CommitNotAuthorized
            | Self::ScopeViolation
            | Self::Token(TokenError::AccessDenied | TokenError::RightsEscalation) => {
                Status::ACCESS_DENIED
            }
            Self::Capacity | Self::Token(TokenError::CaveatCapacity) => Status::NO_SPACE,
            Self::Script(error) => error.status(),
            Self::Cancelled
            | Self::InvalidScope
            | Self::Token(TokenError::Invalid | TokenError::InvalidSignature) => {
                Status::INVALID_ARGUMENT
            }
        }
    }
}

impl From<ghostos_script::Error> for Error {
    fn from(error: ghostos_script::Error) -> Self {
        Self::Script(error)
    }
}

impl From<TokenError> for Error {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
