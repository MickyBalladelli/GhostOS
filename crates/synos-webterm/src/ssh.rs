use syn_shell::MAX_LINE_BYTES;

pub const MAX_SSH_USERNAME_BYTES: usize = 32;
pub const MAX_SSH_PROOF_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl TerminalSize {
    pub const fn new(columns: u16, rows: u16) -> Option<Self> {
        if columns == 0 || rows == 0 {
            None
        } else {
            Some(Self {
                columns,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticatedPrincipal {
    pub identity: u64,
    pub shell_capability: u64,
}

pub trait SshAuthenticator {
    /// Verify the SSH exchange hash proof through the platform crypto/auth service.
    fn authenticate(
        &mut self,
        username: &str,
        public_key: &[u8],
        signature: &[u8],
        exchange_hash: &[u8],
    ) -> Result<AuthenticatedPrincipal, SshError>;
}

pub trait ShellBackend {
    type Handle: Copy;

    fn open(
        &mut self,
        principal: AuthenticatedPrincipal,
        terminal: TerminalSize,
    ) -> Result<Self::Handle, SshError>;
    fn input(&mut self, handle: Self::Handle, bytes: &[u8]) -> Result<usize, SshError>;
    fn resize(&mut self, handle: Self::Handle, terminal: TerminalSize) -> Result<(), SshError>;
    fn output(&mut self, handle: Self::Handle, bytes: &mut [u8]) -> Result<usize, SshError>;
    fn close(&mut self, handle: Self::Handle);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SshSessionId(u64);

impl SshSessionId {
    fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SshError {
    AccessDenied,
    AuthenticationFailed,
    BufferTooLarge,
    InvalidSession,
    InvalidTerminal,
    SessionCapacity,
    WouldBlock,
}

#[derive(Clone, Copy)]
struct Session<H: Copy> {
    generation: u32,
    handle: Option<H>,
    principal: Option<AuthenticatedPrincipal>,
    terminal: Option<TerminalSize>,
}

impl<H: Copy> Session<H> {
    const EMPTY: Self = Self {
        generation: 0,
        handle: None,
        principal: None,
        terminal: None,
    };
}

/// Ring 3 SSH session gate.
///
/// The network-facing SSH transport supplies already-parsed public-key proofs.
/// No shell is allocated until authentication returns a shell capability.
pub struct SshDaemon<A, B: ShellBackend, const SESSION_CAPACITY: usize> {
    authenticator: A,
    shell: B,
    sessions: [Session<B::Handle>; SESSION_CAPACITY],
}

impl<A: SshAuthenticator, B: ShellBackend, const SESSION_CAPACITY: usize>
    SshDaemon<A, B, SESSION_CAPACITY>
{
    pub const fn new(authenticator: A, shell: B) -> Self {
        Self {
            authenticator,
            shell,
            sessions: [Session::EMPTY; SESSION_CAPACITY],
        }
    }

    pub fn open_public_key_session(
        &mut self,
        username: &str,
        public_key: &[u8],
        signature: &[u8],
        exchange_hash: &[u8],
        terminal: TerminalSize,
    ) -> Result<SshSessionId, SshError> {
        if username.is_empty()
            || username.len() > MAX_SSH_USERNAME_BYTES
            || public_key.is_empty()
            || signature.is_empty()
            || public_key.len() > MAX_SSH_PROOF_BYTES
            || signature.len() > MAX_SSH_PROOF_BYTES
            || exchange_hash.is_empty()
            || exchange_hash.len() > 64
        {
            return Err(SshError::AuthenticationFailed);
        }
        if terminal.columns == 0 || terminal.rows == 0 {
            return Err(SshError::InvalidTerminal);
        }
        let principal =
            self.authenticator
                .authenticate(username, public_key, signature, exchange_hash)?;
        if principal.identity == 0 || principal.shell_capability == 0 {
            return Err(SshError::AccessDenied);
        }
        let slot_index = self
            .sessions
            .iter()
            .position(|session| session.handle.is_none())
            .ok_or(SshError::SessionCapacity)?;
        let handle = self.shell.open(principal, terminal)?;
        let slot = &mut self.sessions[slot_index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.handle = Some(handle);
        slot.principal = Some(principal);
        slot.terminal = Some(terminal);
        Ok(SshSessionId::from_parts(slot_index, slot.generation))
    }

    pub fn input(&mut self, session: SshSessionId, bytes: &[u8]) -> Result<usize, SshError> {
        if bytes.len() > MAX_LINE_BYTES {
            return Err(SshError::BufferTooLarge);
        }
        let handle = self.session(session)?.handle.expect("live session");
        self.shell.input(handle, bytes)
    }

    pub fn resize(
        &mut self,
        session: SshSessionId,
        terminal: TerminalSize,
    ) -> Result<(), SshError> {
        if terminal.columns == 0 || terminal.rows == 0 {
            return Err(SshError::InvalidTerminal);
        }
        let handle = self.session(session)?.handle.expect("live session");
        self.shell.resize(handle, terminal)?;
        self.session_mut(session)?.terminal = Some(terminal);
        Ok(())
    }

    pub fn output(&mut self, session: SshSessionId, bytes: &mut [u8]) -> Result<usize, SshError> {
        let handle = self.session(session)?.handle.expect("live session");
        self.shell.output(handle, bytes)
    }

    pub fn close(&mut self, session: SshSessionId) -> Result<(), SshError> {
        let slot = self.session_mut(session)?;
        let handle = slot.handle.take().expect("live session");
        slot.principal = None;
        slot.terminal = None;
        self.shell.close(handle);
        Ok(())
    }

    pub fn active_sessions(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.handle.is_some())
            .count()
    }

    fn session(&self, session: SshSessionId) -> Result<&Session<B::Handle>, SshError> {
        let slot = self
            .sessions
            .get(session.slot())
            .ok_or(SshError::InvalidSession)?;
        if slot.handle.is_none() || slot.generation != session.generation() {
            return Err(SshError::InvalidSession);
        }
        Ok(slot)
    }

    fn session_mut(&mut self, session: SshSessionId) -> Result<&mut Session<B::Handle>, SshError> {
        let slot = self
            .sessions
            .get_mut(session.slot())
            .ok_or(SshError::InvalidSession)?;
        if slot.handle.is_none() || slot.generation != session.generation() {
            return Err(SshError::InvalidSession);
        }
        Ok(slot)
    }
}
