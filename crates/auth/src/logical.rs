use synos_kernel::{
    AddressSpaceId, CapabilityHandle, CapabilityObject, CapabilitySpace, Rights,
};
use synos_system_model::logical::{
    LogicalError, LogicalNameTable, LogicalRights, LogicalScope, LogicalTarget, Principal,
    ResolvedLogicalName,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalNamespace {
    pub scope: LogicalScope,
}

impl LogicalNamespace {
    pub const fn object(self) -> CapabilityObject {
        let (scope, id) = match self.scope {
            LogicalScope::Process(id) => (1, id),
            LogicalScope::Job(id) => (2, id),
            LogicalScope::Group(id) => (3, id),
            LogicalScope::System => (4, 0),
            LogicalScope::Cluster => (5, 0),
        };
        CapabilityObject::LogicalNamespace { scope, id }
    }
}

/// Capability gate in front of the scoped logical-name ACL table.
pub struct CapabilityLogicalNames<
    const ENTRIES: usize,
    const ACLS: usize,
    const CAPABILITIES: usize,
> {
    table: LogicalNameTable<ENTRIES, ACLS>,
    capabilities: CapabilitySpace<CAPABILITIES>,
}

impl<const ENTRIES: usize, const ACLS: usize, const CAPABILITIES: usize>
    CapabilityLogicalNames<ENTRIES, ACLS, CAPABILITIES>
{
    pub const fn new(
        administrator: Principal,
        capabilities: CapabilitySpace<CAPABILITIES>,
    ) -> Self {
        Self {
            table: LogicalNameTable::new(administrator),
            capabilities,
        }
    }

    pub const fn capabilities(&self) -> &CapabilitySpace<CAPABILITIES> {
        &self.capabilities
    }

    pub fn capabilities_mut(&mut self) -> &mut CapabilitySpace<CAPABILITIES> {
        &mut self.capabilities
    }

    pub fn define(
        &mut self,
        caller_space: AddressSpaceId,
        authority: CapabilityHandle,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        self.authorize(caller_space, authority, scope, Rights::WRITE)?;
        self.table.define(caller, scope, name, target)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn grant(
        &mut self,
        caller_space: AddressSpaceId,
        authority: CapabilityHandle,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
        principal: Principal,
        rights: LogicalRights,
    ) -> Result<(), LogicalError> {
        self.authorize(caller_space, authority, scope, Rights::DELEGATE)?;
        self.table.grant(caller, scope, name, principal, rights)
    }

    pub fn delete(
        &mut self,
        caller_space: AddressSpaceId,
        authority: CapabilityHandle,
        caller: Principal,
        scope: LogicalScope,
        name: &str,
    ) -> Result<(), LogicalError> {
        self.authorize(caller_space, authority, scope, Rights::WRITE)?;
        self.table.delete(caller, scope, name)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        &self,
        caller_space: AddressSpaceId,
        authority: CapabilityHandle,
        caller: Principal,
        process: u64,
        job: Option<u64>,
        group: Option<u64>,
        name: &str,
    ) -> Result<ResolvedLogicalName, LogicalError> {
        let resolved = self
            .table
            .resolve_scoped(caller, process, job, group, name)?;
        self.authorize(caller_space, authority, resolved.scope, Rights::READ)?;
        Ok(resolved)
    }

    fn authorize(
        &self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        scope: LogicalScope,
        rights: Rights,
    ) -> Result<(), LogicalError> {
        self.capabilities
            .authorize(
                caller,
                authority,
                LogicalNamespace { scope }.object(),
                rights,
            )
            .map_err(|_| LogicalError::AccessDenied)
    }
}
