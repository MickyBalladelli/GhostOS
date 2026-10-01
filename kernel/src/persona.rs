use core::marker::PhantomData;

pub const MAX_PERSONA_RIGHTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct IdentityId(u64);

impl IdentityId {
    pub const ANONYMOUS: Self = Self(0);

    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RightIdentifier(u64);

impl RightIdentifier {
    pub const SYSTEM_ADMIN: Self = Self(0x5359_5354_454d_4144);
    pub const SYSTEM_OPERATOR: Self = Self(0x5359_5354_454d_4f50);
    pub const SYSTEM_AUDITOR: Self = Self(0x5359_5354_454d_4155);
    pub const SYSTEM_READ_ONLY: Self = Self(0x5359_5354_454d_524f);
    pub const LLM_OPERATOR: Self = Self(0x4c4c_4d5f_4f50_4552);
    pub const NETWORK_INBOUND: Self = Self(0x4e45_545f_494e_4244);
    pub const BATCH_JOB: Self = Self(0x4241_5443_485f_4a4f);

    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaError {
    Full,
    NotFound,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CExecutionPersona {
    identity: u64,
    active: [u64; MAX_PERSONA_RIGHTS],
    disabled: [u64; MAX_PERSONA_RIGHTS],
}

unsafe extern "C" {
    fn ghostos_persona_init(persona: *mut CExecutionPersona, identity: u64);
    fn ghostos_persona_add(persona: *mut CExecutionPersona, right: u64, error: *mut u32) -> bool;
    fn ghostos_persona_disable(persona: *mut CExecutionPersona, right: u64, error: *mut u32) -> bool;
    fn ghostos_persona_enable(persona: *mut CExecutionPersona, right: u64, error: *mut u32) -> bool;
    fn ghostos_persona_drop(persona: *mut CExecutionPersona, right: u64, error: *mut u32) -> bool;
    fn ghostos_persona_has(persona: *const CExecutionPersona, right: u64) -> bool;
    fn ghostos_persona_active_count(persona: *const CExecutionPersona) -> usize;
    fn ghostos_persona_active_at(persona: *const CExecutionPersona, index: usize) -> u64;
}

/// Kernel-owned active rights for one execution context.
///
/// Disabled rights may be restored. Dropped rights are removed permanently.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionPersona {
    raw: CExecutionPersona,
}

impl ExecutionPersona {
    pub const fn anonymous() -> Self {
        Self {
            raw: CExecutionPersona {
                identity: 0,
                active: [0; MAX_PERSONA_RIGHTS],
                disabled: [0; MAX_PERSONA_RIGHTS],
            },
        }
    }

    pub fn new(identity: IdentityId, rights: &[RightIdentifier]) -> Result<Self, PersonaError> {
        if rights.len() > MAX_PERSONA_RIGHTS {
            return Err(PersonaError::Full)
        }
        let mut persona = Self::anonymous();
        // SAFETY: C initializes the plain repr(C) persona value.
        unsafe { ghostos_persona_init(&mut persona.raw, identity.raw()) };
        for right in rights {
            persona.add(*right)?
        }
        Ok(persona)
    }

    pub const fn identity(&self) -> IdentityId {
        IdentityId(self.raw.identity)
    }

    pub fn active_rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        ActiveRights { persona: self, index: 0, count: self.active_count(), marker: PhantomData }
    }

    fn active_count(&self) -> usize {
        // SAFETY: C reads the initialized fixed-size persona arrays.
        unsafe { ghostos_persona_active_count(&self.raw) }
    }

    pub fn has(&self, right: RightIdentifier) -> bool {
        // SAFETY: C reads the initialized fixed-size persona arrays.
        unsafe { ghostos_persona_has(&self.raw, right.raw()) }
    }

    pub fn add(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        // SAFETY: C mutates this uniquely borrowed persona.
        let result = unsafe { ghostos_persona_add(&mut self.raw, right.raw(), core::ptr::null_mut()) };
        result.then_some(()).ok_or(PersonaError::Full)
    }

    pub fn disable(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        self.mutate(right, ghostos_persona_disable)
    }

    pub fn enable(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        self.mutate(right, ghostos_persona_enable)
    }

    pub fn drop_right(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        self.mutate(right, ghostos_persona_drop)
    }

    fn mutate(
        &mut self,
        right: RightIdentifier,
        operation: unsafe extern "C" fn(*mut CExecutionPersona, u64, *mut u32) -> bool,
    ) -> Result<(), PersonaError> {
        let mut error = 0;
        // SAFETY: C mutates this uniquely borrowed persona and writes a valid error code.
        if unsafe { operation(&mut self.raw, right.raw(), &mut error) } {
            Ok(())
        } else if error == 1 {
            Err(PersonaError::Full)
        } else {
            Err(PersonaError::NotFound)
        }
    }
}

struct ActiveRights<'a> {
    persona: &'a ExecutionPersona,
    index: usize,
    count: usize,
    marker: PhantomData<&'a ExecutionPersona>,
}

impl Iterator for ActiveRights<'_> {
    type Item = RightIdentifier;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.count {
            return None
        }
        // SAFETY: index is bounded by C's active-count result and the fixed array size.
        let raw = unsafe { ghostos_persona_active_at(&self.persona.raw, self.index) };
        self.index += 1;
        RightIdentifier::new(raw)
    }
}

impl Default for ExecutionPersona {
    fn default() -> Self {
        Self::anonymous()
    }
}
