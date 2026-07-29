#![no_std]
#![forbid(unsafe_code)]

use syn_script::{
    CowSandbox, LogicalNameControl, SandboxCommandHandler, SandboxDecision, SandboxExecutor,
    SandboxReceipt, Script, ScriptContext, ScriptEngine, ScriptEvent, ToolSchemaExporter,
};
use syn_shell::parser::{CommandRegistration, CommandRegistry};
use synos_auth::{
    CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights,
};
use synos_fabric::NodeId;
use synos_kernel::Rights;
use synos_status::{IntoStatus, Status};
use synos_synfs::SynFs;
use synos_system_model::command::StructuredOutput;

pub const DEFAULT_AGENT_GRANT_CAPACITY: usize = 64;

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
    pub const fn new(
        resource: u64,
        rights: Rights,
        transports: TransportRights,
        max_lifetime_us: u64,
    ) -> Result<Self, Error> {
        if resource == 0 || rights.is_empty() || transports.bits() == 0 || max_lifetime_us == 0 {
            Err(Error::InvalidScope)
        } else {
            Ok(Self {
                resource,
                rights,
                transports,
                max_lifetime_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AgentGrant {
    token: CryptographicCapability,
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
        if parent.issuer != self.issuer
            || parent.resource != scope.resource
            || parent.revocation_epoch != self.revocation_epoch
            || !parent.effective_rights().contains(scope.rights)
            || !parent.effective_transports().contains(scope.transports)
            || lifetime_us == 0
            || lifetime_us > scope.max_lifetime_us
        {
            return Err(Error::ScopeViolation);
        }
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

        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(Error::ScopeViolation)?;
        if expires_at_us > parent.effective_expiry() {
            return Err(Error::ScopeViolation);
        }

        let grant_slot = self
            .grants
            .iter()
            .position(|entry| entry.is_none_or(|grant| grant.expires_at_us <= now_us))
            .ok_or(Error::Capacity)?;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
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
        self.grants[grant_slot] = Some(AgentGrant {
            token,
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
        if required.is_empty() {
            return Err(Error::InvalidScope);
        }
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
        if now_us >= grant.expires_at_us
            || !grant.rights.contains(required)
            || !grant.transports.contains(transport)
        {
            return Err(Error::AccessDenied);
        }
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
        self.grants[slot] = None;
        Ok(())
    }

    pub fn revoke_all(&mut self) {
        self.revocation_epoch = self.revocation_epoch.wrapping_add(1).max(1);
        self.grants.fill(None)
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn active_grants(&self, now_us: u64) -> usize {
        self.grants
            .iter()
            .flatten()
            .filter(|grant| grant.expires_at_us > now_us)
            .count()
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

    pub fn lookup(&self, path: &str) -> Result<synos_synfs::FileVersion, Error> {
        self.sandbox.lookup(path).map_err(Error::Script)
    }

    pub fn read(
        &self,
        path: &str,
        destination: &mut [u8],
    ) -> Result<synos_synfs::ReadResult, Error> {
        self.sandbox.read(path, destination).map_err(Error::Script)
    }

    pub fn finish(self, mode: RunMode) -> Result<RunOutcome, Error> {
        if mode == RunMode::Commit && !self.commit_authorized {
            return Err(Error::CommitNotAuthorized);
        }
        let decision = if mode == RunMode::Commit && self.status.is_success() {
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
/// runs procedures entirely through a SynFS transaction.
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
        let mut required = request.task_rights.union(Rights::EXECUTE);
        if request.mode == RunMode::Commit {
            required = required.union(Rights::WRITE)
        }
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
        command: syn_shell::parser::CommandCall,
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
    Script(syn_script::Error),
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

impl From<syn_script::Error> for Error {
    fn from(error: syn_script::Error) -> Self {
        Self::Script(error)
    }
}

impl From<TokenError> for Error {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
