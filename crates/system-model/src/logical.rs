use crate::logical_fast::LogicalFastPath;
use crate::{Error as ModelError, LogicalName};
use synos_status::{IntoStatus, Severity, Status, facility};

pub const MAX_LOGICAL_VALUE_BYTES: usize = 192;
pub const DEFAULT_LOGICAL_CAPACITY: usize = 128;
pub const DEFAULT_ACL_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Principal(u64);

impl Principal {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalScope {
    Process(u64),
    Job(u64),
    Group(u64),
    System,
    Cluster,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalTargetKind {
    File,
    Device,
    IpcChannel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalTarget {
    pub kind: LogicalTargetKind,
    bytes: [u8; MAX_LOGICAL_VALUE_BYTES],
    len: u8,
}

impl LogicalTarget {
    pub fn new(kind: LogicalTargetKind, value: &str) -> Result<Self, LogicalError> {
        let source = value.as_bytes();
        if source.is_empty() || source.len() > MAX_LOGICAL_VALUE_BYTES || source.contains(&0) {
            return Err(LogicalError::InvalidTarget);
        }
        let mut bytes = [0; MAX_LOGICAL_VALUE_BYTES];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self {
            kind,
            bytes,
            len: source.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("LogicalTarget invariant")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct LogicalRights(u8);

impl LogicalRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const DEFINE: Self = Self(1 << 1);
    pub const DELETE: Self = Self(1 << 2);
    pub const CONTROL: Self = Self(1 << 3);
    pub const ALL: Self = Self((1 << 4) - 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AccessEntry {
    principal: Principal,
    rights: LogicalRights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LogicalEntry<const ACLS: usize> {
    scope: LogicalScope,
    name: LogicalName,
    target: LogicalTarget,
    owner: Principal,
    acl: [Option<AccessEntry>; ACLS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedLogicalName {
    pub scope: LogicalScope,
    pub name: LogicalName,
    pub target: LogicalTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalError {
    AccessDenied,
    AclFull,
    InvalidName,
    InvalidRights,
    InvalidTarget,
    NotFound,
    TableFull,
}

impl IntoStatus for LogicalError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::NotFound => Status::NOT_FOUND,
            Self::AclFull | Self::TableFull => Status::NO_SPACE,
            Self::InvalidName | Self::InvalidRights | Self::InvalidTarget => {
                Status::new(Severity::Error, facility::LOGICAL_NAME, 1, 0)
                    .expect("valid logical-name status")
            }
        }
    }
}

/// Scoped logical-name dictionary with per-entry ACLs.
///
/// Resolution walks process, group, system, then cluster scope. A matching
/// private entry never falls through to a less-specific alias.
pub struct LogicalNameTable<
    const CAPACITY: usize = DEFAULT_LOGICAL_CAPACITY,
    const ACLS: usize = DEFAULT_ACL_CAPACITY,
> {
    administrator: Principal,
    entries: [Option<LogicalEntry<ACLS>>; CAPACITY],
}

impl<const CAPACITY: usize, const ACLS: usize> LogicalNameTable<CAPACITY, ACLS> {
    pub const fn new(administrator: Principal) -> Self {
        Self {
            administrator,
            entries: [None; CAPACITY],
        }
    }

    pub fn define(
        &mut self,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        self.authorize_scope_creation(caller, scope)?;
        let name = LogicalName::new(name).map_err(map_name_error)?;
        if let Some(index) = self.find(scope, name) {
            self.authorize(index, caller, LogicalRights::DEFINE)?;
            self.entries[index].as_mut().expect("entry exists").target = target;
            return Ok(());
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(LogicalError::TableFull)?;
        *slot = Some(LogicalEntry {
            scope,
            name,
            target,
            owner: caller,
            acl: [None; ACLS],
        });
        Ok(())
    }

    pub fn grant(
        &mut self,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
        principal: Principal,
        rights: LogicalRights,
    ) -> Result<(), LogicalError> {
        if rights.is_empty() {
            return Err(LogicalError::InvalidRights);
        }
        let name = LogicalName::new(name).map_err(map_name_error)?;
        let index = self.find(scope, name).ok_or(LogicalError::NotFound)?;
        self.authorize(index, caller, LogicalRights::CONTROL)?;
        let entry = self.entries[index].as_mut().expect("entry exists");
        if let Some(access) = entry
            .acl
            .iter_mut()
            .flatten()
            .find(|access| access.principal == principal)
        {
            access.rights = rights;
            return Ok(());
        }
        let slot = entry
            .acl
            .iter_mut()
            .find(|access| access.is_none())
            .ok_or(LogicalError::AclFull)?;
        *slot = Some(AccessEntry { principal, rights });
        Ok(())
    }

    pub fn delete(
        &mut self,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
    ) -> Result<(), LogicalError> {
        let name = LogicalName::new(name).map_err(map_name_error)?;
        let index = self.find(scope, name).ok_or(LogicalError::NotFound)?;
        self.authorize(index, caller, LogicalRights::DELETE)?;
        self.entries[index] = None;
        Ok(())
    }

    pub fn resolve(
        &self,
        caller: Principal,
        process: u64,
        group: Option<u64>,
        name: &str,
    ) -> Result<ResolvedLogicalName, LogicalError> {
        self.resolve_scoped(caller, process, None, group, name)
    }

    pub fn resolve_scoped(
        &self,
        caller: Principal,
        process: u64,
        job: Option<u64>,
        group: Option<u64>,
        name: &str,
    ) -> Result<ResolvedLogicalName, LogicalError> {
        let name = LogicalName::new(name).map_err(map_name_error)?;
        if let Some(result) = self.resolve_at(caller, LogicalScope::Process(process), name)? {
            return Ok(result);
        }
        if let Some(job) = job {
            if let Some(result) = self.resolve_at(caller, LogicalScope::Job(job), name)? {
                return Ok(result);
            }
        }
        if let Some(group) = group {
            if let Some(result) = self.resolve_at(caller, LogicalScope::Group(group), name)? {
                return Ok(result);
            }
        }
        if let Some(result) = self.resolve_at(caller, LogicalScope::System, name)? {
            return Ok(result);
        }
        self.resolve_at(caller, LogicalScope::Cluster, name)?
            .ok_or(LogicalError::NotFound)
    }

    /// Publish authorized process and system aliases into atomic read-only
    /// pages used by the Ring 3 resolver fast path.
    pub fn refresh_fast_path<const PROCESS: usize, const SYSTEM: usize>(
        &self,
        caller: Principal,
        process: u64,
        fast_path: &LogicalFastPath<PROCESS, SYSTEM>,
    ) -> Result<u64, LogicalError> {
        fast_path.begin_publish(process);
        let result = self.entries.iter().flatten().try_for_each(|entry| {
            let visible = match entry.scope {
                LogicalScope::Process(owner) => owner == process,
                LogicalScope::System => true,
                LogicalScope::Job(_) | LogicalScope::Group(_) | LogicalScope::Cluster => false,
            };
            if !visible {
                return Ok(());
            }
            let index = self
                .find(entry.scope, entry.name)
                .ok_or(LogicalError::NotFound)?;
            if self.authorize(index, caller, LogicalRights::READ).is_err() {
                return Ok(());
            }
            match entry.scope {
                LogicalScope::Process(_) => fast_path.insert_process(entry.name, entry.target),
                LogicalScope::System => fast_path.insert_system(entry.name, entry.target),
                LogicalScope::Job(_) | LogicalScope::Group(_) | LogicalScope::Cluster => Ok(()),
            }
        });
        if result.is_err() {
            fast_path.abort_publish()
        } else {
            fast_path.finish_publish()
        }
        result.map(|()| fast_path.epoch())
    }

    fn resolve_at(
        &self,
        caller: Principal,
        scope: LogicalScope,
        name: LogicalName,
    ) -> Result<Option<ResolvedLogicalName>, LogicalError> {
        let Some(index) = self.find(scope, name) else {
            return Ok(None);
        };
        self.authorize(index, caller, LogicalRights::READ)?;
        let entry = self.entries[index].expect("entry exists");
        Ok(Some(ResolvedLogicalName {
            scope,
            name,
            target: entry.target,
        }))
    }

    fn authorize_scope_creation(
        &self,
        caller: Principal,
        scope: LogicalScope,
    ) -> Result<(), LogicalError> {
        let allowed = match scope {
            LogicalScope::Process(process) => caller.raw() == process,
            LogicalScope::Job(job) => caller.raw() == job || caller == self.administrator,
            LogicalScope::Group(_) | LogicalScope::System | LogicalScope::Cluster => {
                caller == self.administrator
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(LogicalError::AccessDenied)
        }
    }

    fn authorize(
        &self,
        index: usize,
        caller: Principal,
        required: LogicalRights,
    ) -> Result<(), LogicalError> {
        let entry = self.entries[index].expect("entry exists");
        if entry.owner == caller
            || entry
                .acl
                .iter()
                .flatten()
                .any(|access| access.principal == caller && access.rights.contains(required))
        {
            Ok(())
        } else {
            Err(LogicalError::AccessDenied)
        }
    }

    fn find(&self, scope: LogicalScope, name: LogicalName) -> Option<usize> {
        self.entries.iter().position(|entry| {
            entry.is_some_and(|entry| entry.scope == scope && names_equal(entry.name, name))
        })
    }
}

fn names_equal(left: LogicalName, right: LogicalName) -> bool {
    left.as_str().eq_ignore_ascii_case(right.as_str())
}

fn map_name_error(_: ModelError) -> LogicalError {
    LogicalError::InvalidName
}
