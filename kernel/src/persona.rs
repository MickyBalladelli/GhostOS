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

/// Kernel-owned active rights for one execution context.
///
/// Disabled rights may be restored. Dropped rights are removed permanently,
/// which lets a process shed authority before running untrusted code.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionPersona {
    identity: IdentityId,
    active: [Option<RightIdentifier>; MAX_PERSONA_RIGHTS],
    disabled: [Option<RightIdentifier>; MAX_PERSONA_RIGHTS],
}

impl ExecutionPersona {
    pub const fn anonymous() -> Self {
        Self {
            identity: IdentityId::ANONYMOUS,
            active: [None; MAX_PERSONA_RIGHTS],
            disabled: [None; MAX_PERSONA_RIGHTS],
        }
    }

    pub fn new(identity: IdentityId, rights: &[RightIdentifier]) -> Result<Self, PersonaError> {
        if rights.len() > MAX_PERSONA_RIGHTS {
            return Err(PersonaError::Full)
        }
        let mut persona = Self {
            identity,
            active: [None; MAX_PERSONA_RIGHTS],
            disabled: [None; MAX_PERSONA_RIGHTS],
        };
        for right in rights {
            persona.add(*right)?
        }
        Ok(persona)
    }

    pub const fn identity(&self) -> IdentityId {
        self.identity
    }

    pub fn active_rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        self.active.iter().flatten().copied()
    }

    pub fn has(&self, right: RightIdentifier) -> bool {
        self.active.contains(&Some(right))
    }

    pub fn add(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        if self.has(right) || self.disabled.contains(&Some(right)) {
            return Ok(())
        }
        let slot = self
            .active
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PersonaError::Full)?;
        *slot = Some(right);
        Ok(())
    }

    pub fn disable(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        let active = self
            .active
            .iter_mut()
            .find(|entry| **entry == Some(right))
            .ok_or(PersonaError::NotFound)?;
        let disabled = self
            .disabled
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PersonaError::Full)?;
        *active = None;
        *disabled = Some(right);
        Ok(())
    }

    pub fn enable(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        let disabled = self
            .disabled
            .iter_mut()
            .find(|entry| **entry == Some(right))
            .ok_or(PersonaError::NotFound)?;
        let active = self
            .active
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PersonaError::Full)?;
        *disabled = None;
        *active = Some(right);
        Ok(())
    }

    pub fn drop_right(&mut self, right: RightIdentifier) -> Result<(), PersonaError> {
        if let Some(entry) = self
            .active
            .iter_mut()
            .find(|entry| **entry == Some(right))
        {
            *entry = None;
            return Ok(())
        }
        if let Some(entry) = self
            .disabled
            .iter_mut()
            .find(|entry| **entry == Some(right))
        {
            *entry = None;
            return Ok(())
        }
        Err(PersonaError::NotFound)
    }
}

impl Default for ExecutionPersona {
    fn default() -> Self {
        Self::anonymous()
    }
}
