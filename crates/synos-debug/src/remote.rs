//! Capability-authenticated GDB sessions for remote debugging.
//!
//! The transport remains outside this crate. A caller authenticates a remote
//! administrator, receives a signed capability for one process resource, and
//! feeds the resulting byte stream to [`RemoteGdbSession`]. The session checks
//! the capability before every GDB operation, so expiry and revocation take
//! effect without closing over a stale authorization decision.

use synos_auth::{
    CryptographicCapability, RemoteAdminSession, RemoteTokenError, RemoteTokenIssuer,
};
use synos_init::ProcessId;
use synos_kernel::Rights;
use synos_status::{IntoStatus, Status};

use crate::{Error, gdb::{DebugAuthority, DebugOperation, DebugToken, DebugRuntime, GdbStub}};

pub trait DebugClock {
    fn now_us(&self) -> u64;
}

impl DebugClock for u64 {
    fn now_us(&self) -> u64 {
        *self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteDebugError {
    AccessDenied,
    InvalidCapability,
    TargetMismatch,
}

impl From<RemoteTokenError> for RemoteDebugError {
    fn from(_error: RemoteTokenError) -> Self {
        Self::AccessDenied
    }
}

impl IntoStatus for RemoteDebugError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::InvalidCapability | Self::TargetMismatch => Status::INVALID_ARGUMENT,
        }
    }
}

/// Authorization for one remote GDB connection.
pub struct RemoteDebugAuthority<'a, C, const SCOPES: usize> {
    issuer: &'a RemoteTokenIssuer<SCOPES>,
    session: RemoteAdminSession,
    capability: CryptographicCapability,
    target: ProcessId,
    clock: &'a C,
}

impl<'a, C, const SCOPES: usize> RemoteDebugAuthority<'a, C, SCOPES>
where
    C: DebugClock,
{
    pub fn new(
        issuer: &'a RemoteTokenIssuer<SCOPES>,
        session: RemoteAdminSession,
        capability: CryptographicCapability,
        target: ProcessId,
        clock: &'a C,
    ) -> Result<Self, RemoteDebugError> {
        if capability.resource != target.raw() || capability.nonce == 0 {
            return Err(RemoteDebugError::TargetMismatch)
        }
        issuer
            .authorize(
                &capability,
                &session,
                Rights::DEBUG,
                clock.now_us(),
            )
            .map_err(RemoteDebugError::from)?;
        Ok(Self {
            issuer,
            session,
            capability,
            target,
            clock,
        })
    }

    pub const fn target(&self) -> ProcessId {
        self.target
    }

    pub const fn capability(&self) -> CryptographicCapability {
        self.capability
    }

    fn required_rights(operation: DebugOperation) -> Rights {
        match operation {
            DebugOperation::Read => Rights::DEBUG.union(Rights::READ),
            DebugOperation::Write => Rights::DEBUG.union(Rights::WRITE),
            DebugOperation::Control => Rights::DEBUG,
        }
    }
}

impl<C, const SCOPES: usize> DebugAuthority for RemoteDebugAuthority<'_, C, SCOPES>
where
    C: DebugClock,
{
    fn permits(&self, token: DebugToken, operation: DebugOperation) -> bool {
        token.raw() == self.capability.nonce
            && self
                .issuer
                .authorize(
                    &self.capability,
                    &self.session,
                    Self::required_rights(operation),
                    self.clock.now_us(),
                )
                .is_ok()
    }
}

pub struct RemoteGdbSession<'a, R, C, const SCOPES: usize> {
    stub: GdbStub<R, RemoteDebugAuthority<'a, C, SCOPES>>,
}

impl<'a, R, C, const SCOPES: usize> RemoteGdbSession<'a, R, C, SCOPES>
where
    R: DebugRuntime,
    C: DebugClock,
{
    pub fn open(
        runtime: R,
        issuer: &'a RemoteTokenIssuer<SCOPES>,
        session: RemoteAdminSession,
        capability: CryptographicCapability,
        target: ProcessId,
        clock: &'a C,
    ) -> Result<Self, RemoteDebugError> {
        let authority = RemoteDebugAuthority::new(issuer, session, capability, target, clock)?;
        let token = DebugToken::new(capability.nonce).ok_or(RemoteDebugError::InvalidCapability)?;
        Ok(Self {
            stub: GdbStub::new(runtime, authority, token),
        })
    }

    pub fn open_wire(
        runtime: R,
        issuer: &'a RemoteTokenIssuer<SCOPES>,
        session: RemoteAdminSession,
        wire: &[u8],
        target: ProcessId,
        clock: &'a C,
    ) -> Result<Self, RemoteDebugError> {
        if wire.len() != CryptographicCapability::WIRE_BYTES {
            return Err(RemoteDebugError::InvalidCapability)
        }
        let mut bytes = [0; CryptographicCapability::WIRE_BYTES];
        bytes.copy_from_slice(wire);
        let capability = CryptographicCapability::decode(bytes)
            .map_err(|_error| RemoteDebugError::InvalidCapability)?;
        Self::open(runtime, issuer, session, capability, target, clock)
    }

    pub fn ingest(&mut self, input: &[u8], output: &mut [u8]) -> Result<Option<usize>, Error> {
        self.stub.ingest(input, output)
    }

    pub fn runtime(&self) -> &R {
        self.stub.runtime()
    }

    pub fn runtime_mut(&mut self) -> &mut R {
        self.stub.runtime_mut()
    }

    pub const fn is_detached(&self) -> bool {
        self.stub.is_detached()
    }
}
