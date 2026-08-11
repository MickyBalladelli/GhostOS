#![no_std]
#![forbid(unsafe_code)]

pub const MAX_POLICY_PRINCIPALS: usize = 64;
pub const MAX_POLICY_OBJECTS: usize = 128;
pub const MAX_POLICY_BINDINGS: usize = 256;
pub const MAX_SIMULATION_AFFECTED: usize = MAX_POLICY_OBJECTS;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct PrincipalId([u8; 32]);

impl PrincipalId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn from_u64(value: u64) -> Self {
        let mut bytes = [0; 32];
        bytes[24] = (value >> 56) as u8;
        bytes[25] = (value >> 48) as u8;
        bytes[26] = (value >> 40) as u8;
        bytes[27] = (value >> 32) as u8;
        bytes[28] = (value >> 24) as u8;
        bytes[29] = (value >> 16) as u8;
        bytes[30] = (value >> 8) as u8;
        bytes[31] = value as u8;
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn from_u64(value: u64) -> Self {
        let mut bytes = [0; 32];
        bytes[24] = (value >> 56) as u8;
        bytes[25] = (value >> 48) as u8;
        bytes[26] = (value >> 40) as u8;
        bytes[27] = (value >> 32) as u8;
        bytes[28] = (value >> 24) as u8;
        bytes[29] = (value >> 16) as u8;
        bytes[30] = (value >> 8) as u8;
        bytes[31] = value as u8;
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ObjectKind {
    Capability = 1,
    FirewallRule = 2,
    Network = 3,
    Package = 4,
    PackageProcess = 5,
    Cluster = 6,
    ClusterMember = 7,
    Update = 8,
    System = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrincipalRecord {
    pub id: PrincipalId,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectRecord {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub owner: Option<PrincipalId>,
    pub parent: Option<ObjectId>,
    pub revision: u64,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub principal: PrincipalId,
    pub object: ObjectId,
    pub rights: u64,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Capability,
    Firewall,
    PackageActivation,
    ClusterMembership,
    UpdateRollout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityChange {
    pub principal: PrincipalId,
    pub object: ObjectId,
    pub before_rights: u64,
    pub after_rights: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirewallChange {
    pub rule: ObjectId,
    pub applies_to: ObjectId,
    pub before_revision: u64,
    pub after_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageActivationChange {
    pub package: ObjectId,
    pub before_revision: u64,
    pub after_revision: u64,
    pub activate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMembershipChange {
    pub cluster: ObjectId,
    pub member: ObjectId,
    pub before_active: bool,
    pub after_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateRolloutChange {
    pub update: ObjectId,
    pub from_revision: u64,
    pub to_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyChange {
    Capability(CapabilityChange),
    Firewall(FirewallChange),
    PackageActivation(PackageActivationChange),
    ClusterMembership(ClusterMembershipChange),
    UpdateRollout(UpdateRolloutChange),
}

impl PolicyChange {
    pub const fn kind(self) -> ChangeKind {
        match self {
            Self::Capability(_) => ChangeKind::Capability,
            Self::Firewall(_) => ChangeKind::Firewall,
            Self::PackageActivation(_) => ChangeKind::PackageActivation,
            Self::ClusterMembership(_) => ChangeKind::ClusterMembership,
            Self::UpdateRollout(_) => ChangeKind::UpdateRollout,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimulationError {
    Capacity,
    UnknownPrincipal,
    UnknownObject,
    InvalidObjectKind,
    InvalidChange,
    StaleSnapshot,
    TooManyAffected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AffectedPrincipal {
    pub id: PrincipalId,
    pub reason: ChangeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AffectedObject {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub reason: ChangeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SimulationReport {
    pub change: PolicyChange,
    pub before_epoch: u64,
    pub after_epoch: u64,
    pub before_fingerprint: u64,
    pub after_fingerprint: u64,
    pub changed: bool,
    principals: [Option<AffectedPrincipal>; MAX_SIMULATION_AFFECTED],
    principal_count: usize,
    objects: [Option<AffectedObject>; MAX_SIMULATION_AFFECTED],
    object_count: usize,
}

impl SimulationReport {
    pub fn affected_principals(&self) -> impl Iterator<Item = AffectedPrincipal> + '_ {
        self.principals[..self.principal_count].iter().flatten().copied()
    }

    pub fn affected_objects(&self) -> impl Iterator<Item = AffectedObject> + '_ {
        self.objects[..self.object_count].iter().flatten().copied()
    }

    pub const fn principal_count(&self) -> usize {
        self.principal_count
    }

    pub const fn object_count(&self) -> usize {
        self.object_count
    }

    pub const fn is_read_only(&self) -> bool {
        self.before_epoch == self.after_epoch
            && self.before_fingerprint == self.after_fingerprint
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicySnapshot<
    const PRINCIPALS: usize = MAX_POLICY_PRINCIPALS,
    const OBJECTS: usize = MAX_POLICY_OBJECTS,
    const BINDINGS: usize = MAX_POLICY_BINDINGS,
> {
    epoch: u64,
    principals: [Option<PrincipalRecord>; PRINCIPALS],
    objects: [Option<ObjectRecord>; OBJECTS],
    bindings: [Option<Binding>; BINDINGS],
}

impl<const PRINCIPALS: usize, const OBJECTS: usize, const BINDINGS: usize>
    PolicySnapshot<PRINCIPALS, OBJECTS, BINDINGS>
{
    pub const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            principals: [None; PRINCIPALS],
            objects: [None; OBJECTS],
            bindings: [None; BINDINGS],
        }
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn add_principal(&mut self, principal: PrincipalId) -> Result<(), SimulationError> {
        if self.principals.iter().flatten().any(|entry| entry.id == principal) {
            return Ok(())
        }
        let slot = self
            .principals
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SimulationError::Capacity)?;
        *slot = Some(PrincipalRecord { id: principal, active: true });
        Ok(())
    }

    pub fn add_object(&mut self, object: ObjectRecord) -> Result<(), SimulationError> {
        if self.objects.iter().flatten().any(|entry| entry.id == object.id) {
            return Err(SimulationError::InvalidChange)
        }
        let slot = self
            .objects
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SimulationError::Capacity)?;
        *slot = Some(object);
        Ok(())
    }

    pub fn bind(&mut self, binding: Binding) -> Result<(), SimulationError> {
        self.require_principal(binding.principal)?;
        self.require_object(binding.object)?;
        if let Some(existing) = self.bindings.iter_mut().flatten().find(|entry| {
            entry.principal == binding.principal && entry.object == binding.object
        }) {
            *existing = binding;
            return Ok(())
        }
        let slot = self
            .bindings
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SimulationError::Capacity)?;
        *slot = Some(binding);
        Ok(())
    }

    pub fn simulate(&self, change: PolicyChange) -> Result<SimulationReport, SimulationError> {
        let before_fingerprint = self.fingerprint();
        let mut report = SimulationReport {
            change,
            before_epoch: self.epoch,
            after_epoch: self.epoch,
            before_fingerprint,
            after_fingerprint: before_fingerprint,
            changed: false,
            principals: [None; MAX_SIMULATION_AFFECTED],
            principal_count: 0,
            objects: [None; MAX_SIMULATION_AFFECTED],
            object_count: 0,
        };
        match change {
            PolicyChange::Capability(change) => self.simulate_capability(change, &mut report)?,
            PolicyChange::Firewall(change) => self.simulate_firewall(change, &mut report)?,
            PolicyChange::PackageActivation(change) => {
                self.simulate_package(change, &mut report)?
            }
            PolicyChange::ClusterMembership(change) => {
                self.simulate_membership(change, &mut report)?
            }
            PolicyChange::UpdateRollout(change) => self.simulate_update(change, &mut report)?,
        }
        Ok(report)
    }

    fn simulate_capability(
        &self,
        change: CapabilityChange,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        self.require_principal(change.principal)?;
        let object = self.require_object(change.object)?;
        let current = self
            .binding(change.principal, change.object)
            .map_or(0, |binding| binding.rights);
        if current != change.before_rights {
            return Err(SimulationError::StaleSnapshot)
        }
        if change.before_rights != change.after_rights {
            report.changed = true;
            report.add_principal(AffectedPrincipal { id: change.principal, reason: ChangeKind::Capability })?;
            report.add_object(AffectedObject { id: object.id, kind: object.kind, reason: ChangeKind::Capability })?;
        }
        Ok(())
    }

    fn simulate_firewall(
        &self,
        change: FirewallChange,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        let rule = self.require_kind(change.rule, ObjectKind::FirewallRule)?;
        let applies_to = self.require_kind(change.applies_to, ObjectKind::Network)?;
        if rule.revision != change.before_revision {
            return Err(SimulationError::StaleSnapshot)
        }
        if change.before_revision == change.after_revision {
            return Ok(())
        }
        report.changed = true;
        report.add_object(AffectedObject { id: rule.id, kind: rule.kind, reason: ChangeKind::Firewall })?;
        report.add_object(AffectedObject { id: applies_to.id, kind: applies_to.kind, reason: ChangeKind::Firewall })?;
        self.add_bound_principals(change.applies_to, ChangeKind::Firewall, report)?;
        Ok(())
    }

    fn simulate_package(
        &self,
        change: PackageActivationChange,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        let package = self.require_kind(change.package, ObjectKind::Package)?;
        if change.after_revision == 0 {
            return Err(SimulationError::InvalidChange)
        }
        if package.revision != change.before_revision {
            return Err(SimulationError::StaleSnapshot)
        }
        if change.before_revision == change.after_revision && package.active == change.activate {
            return Ok(())
        }
        report.changed = true;
        report.add_object(AffectedObject { id: package.id, kind: package.kind, reason: ChangeKind::PackageActivation })?;
        self.add_descendants(change.package, ChangeKind::PackageActivation, report)?;
        self.add_bound_principals(change.package, ChangeKind::PackageActivation, report)?;
        Ok(())
    }

    fn simulate_membership(
        &self,
        change: ClusterMembershipChange,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        self.require_kind(change.cluster, ObjectKind::Cluster)?;
        let member = self.require_kind(change.member, ObjectKind::ClusterMember)?;
        if member.active != change.before_active {
            return Err(SimulationError::StaleSnapshot)
        }
        if change.before_active == change.after_active {
            return Ok(())
        }
        report.changed = true;
        report.add_object(AffectedObject { id: change.cluster, kind: ObjectKind::Cluster, reason: ChangeKind::ClusterMembership })?;
        report.add_object(AffectedObject { id: member.id, kind: member.kind, reason: ChangeKind::ClusterMembership })?;
        self.add_bound_principals(change.member, ChangeKind::ClusterMembership, report)?;
        Ok(())
    }

    fn simulate_update(
        &self,
        change: UpdateRolloutChange,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        let update = self.require_kind(change.update, ObjectKind::Update)?;
        if update.revision != change.from_revision {
            return Err(SimulationError::StaleSnapshot)
        }
        if change.from_revision == change.to_revision {
            return Ok(())
        }
        if change.to_revision == 0 {
            return Err(SimulationError::InvalidChange)
        }
        report.changed = true;
        report.add_object(AffectedObject { id: change.update, kind: ObjectKind::Update, reason: ChangeKind::UpdateRollout })?;
        for object in self.objects.iter().flatten() {
            if object.active && object.revision == change.from_revision && object.id != change.update {
                report.add_object(AffectedObject { id: object.id, kind: object.kind, reason: ChangeKind::UpdateRollout })?;
                self.add_bound_principals(object.id, ChangeKind::UpdateRollout, report)?;
            }
        }
        Ok(())
    }

    fn add_bound_principals(
        &self,
        object: ObjectId,
        reason: ChangeKind,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        for binding in self.bindings.iter().flatten() {
            if binding.object == object && binding.active {
                report.add_principal(AffectedPrincipal { id: binding.principal, reason })?;
            }
        }
        Ok(())
    }

    fn add_descendants(
        &self,
        parent: ObjectId,
        reason: ChangeKind,
        report: &mut SimulationReport,
    ) -> Result<(), SimulationError> {
        for object in self.objects.iter().flatten() {
            if object.parent == Some(parent) {
                report.add_object(AffectedObject { id: object.id, kind: object.kind, reason })?;
                self.add_bound_principals(object.id, reason, report)?;
            }
        }
        Ok(())
    }

    fn require_principal(&self, id: PrincipalId) -> Result<PrincipalRecord, SimulationError> {
        self.principals
            .iter()
            .flatten()
            .find(|principal| principal.id == id)
            .copied()
            .ok_or(SimulationError::UnknownPrincipal)
    }

    fn require_object(&self, id: ObjectId) -> Result<ObjectRecord, SimulationError> {
        self.objects
            .iter()
            .flatten()
            .find(|object| object.id == id)
            .copied()
            .ok_or(SimulationError::UnknownObject)
    }

    fn require_kind(&self, id: ObjectId, kind: ObjectKind) -> Result<ObjectRecord, SimulationError> {
        let object = self.require_object(id)?;
        if object.kind != kind {
            return Err(SimulationError::InvalidObjectKind)
        }
        Ok(object)
    }

    fn binding(&self, principal: PrincipalId, object: ObjectId) -> Option<Binding> {
        self.bindings
            .iter()
            .flatten()
            .find(|binding| binding.principal == principal && binding.object == object)
            .copied()
    }

    fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325;
        hash = mix(hash, self.epoch);
        for principal in self.principals.iter().flatten() {
            for byte in principal.id.as_bytes() {
                hash = mix(hash, byte as u64)
            }
            hash = mix(hash, principal.active as u64);
        }
        for object in self.objects.iter().flatten() {
            for byte in object.id.as_bytes() {
                hash = mix(hash, byte as u64)
            }
            hash = mix(hash, object.kind as u64);
            if let Some(owner) = object.owner {
                for byte in owner.as_bytes() {
                    hash = mix(hash, byte as u64)
                }
            }
            if let Some(parent) = object.parent {
                for byte in parent.as_bytes() {
                    hash = mix(hash, byte as u64)
                }
            }
            hash = mix(hash, object.revision);
            hash = mix(hash, object.active as u64);
        }
        for binding in self.bindings.iter().flatten() {
            for byte in binding.principal.as_bytes() {
                hash = mix(hash, byte as u64)
            }
            for byte in binding.object.as_bytes() {
                hash = mix(hash, byte as u64)
            }
            hash = mix(hash, binding.rights);
            hash = mix(hash, binding.active as u64);
        }
        hash
    }
}

impl SimulationReport {
    fn add_principal(&mut self, affected: AffectedPrincipal) -> Result<(), SimulationError> {
        if self.principals[..self.principal_count]
            .iter()
            .flatten()
            .any(|entry| entry.id == affected.id)
        {
            return Ok(())
        }
        if self.principal_count == MAX_SIMULATION_AFFECTED {
            return Err(SimulationError::TooManyAffected)
        }
        self.principals[self.principal_count] = Some(affected);
        self.principal_count += 1;
        Ok(())
    }

    fn add_object(&mut self, affected: AffectedObject) -> Result<(), SimulationError> {
        if self.objects[..self.object_count]
            .iter()
            .flatten()
            .any(|entry| entry.id == affected.id)
        {
            return Ok(())
        }
        if self.object_count == MAX_SIMULATION_AFFECTED {
            return Err(SimulationError::TooManyAffected)
        }
        self.objects[self.object_count] = Some(affected);
        self.object_count += 1;
        Ok(())
    }
}

fn mix(mut hash: u64, value: u64) -> u64 {
    hash ^= value;
    hash.wrapping_mul(0x100000001b3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> PolicySnapshot<8, 16, 16> {
        let mut snapshot = PolicySnapshot::new(7);
        snapshot.add_principal(PrincipalId::from_u64(1)).unwrap();
        snapshot
            .add_object(ObjectRecord {
                id: ObjectId::from_u64(2),
                kind: ObjectKind::Capability,
                owner: Some(PrincipalId::from_u64(1)),
                parent: None,
                revision: 3,
                active: true,
            })
            .unwrap();
        snapshot
            .bind(Binding {
                principal: PrincipalId::from_u64(1),
                object: ObjectId::from_u64(2),
                rights: 1,
                active: true,
            })
            .unwrap();
        snapshot
    }

    #[test]
    fn simulation_does_not_mutate_snapshot() {
        let snapshot = snapshot();
        let epoch = snapshot.epoch();
        let report = snapshot
            .simulate(PolicyChange::Capability(CapabilityChange {
                principal: PrincipalId::from_u64(1),
                object: ObjectId::from_u64(2),
                before_rights: 1,
                after_rights: 0,
            }))
            .unwrap();
        assert!(report.changed);
        assert!(report.is_read_only());
        assert_eq!(snapshot.epoch(), epoch);
        assert_eq!(snapshot.binding(PrincipalId::from_u64(1), ObjectId::from_u64(2)).unwrap().rights, 1);
    }

    #[test]
    fn package_simulation_reports_children_and_principals() {
        let mut snapshot = snapshot();
        snapshot
            .add_object(ObjectRecord {
                id: ObjectId::from_u64(3),
                kind: ObjectKind::Package,
                owner: None,
                parent: None,
                revision: 4,
                active: true,
            })
            .unwrap();
        snapshot
            .add_object(ObjectRecord {
                id: ObjectId::from_u64(4),
                kind: ObjectKind::PackageProcess,
                owner: Some(PrincipalId::from_u64(1)),
                parent: Some(ObjectId::from_u64(3)),
                revision: 4,
                active: true,
            })
            .unwrap();
        snapshot
            .bind(Binding {
                principal: PrincipalId::from_u64(1),
                object: ObjectId::from_u64(3),
                rights: 1,
                active: true,
            })
            .unwrap();
        let report = snapshot
            .simulate(PolicyChange::PackageActivation(PackageActivationChange {
                package: ObjectId::from_u64(3),
                before_revision: 4,
                after_revision: 5,
                activate: false,
            }))
            .unwrap();
        assert_eq!(report.object_count(), 2);
        assert_eq!(report.principal_count(), 1);
    }

    #[test]
    fn simulation_covers_firewall_membership_and_rollout() {
        let principal = PrincipalId::from_u64(1);
        let mut snapshot = PolicySnapshot::<8, 16, 16>::new(9);
        snapshot.add_principal(principal).unwrap();
        for (id, kind, parent, revision, active) in [
            (10, ObjectKind::FirewallRule, None, 1, true),
            (11, ObjectKind::Network, None, 1, true),
            (20, ObjectKind::Cluster, None, 1, true),
            (21, ObjectKind::ClusterMember, Some(20), 1, true),
            (30, ObjectKind::Update, None, 1, true),
        ] {
            snapshot
                .add_object(ObjectRecord {
                    id: ObjectId::from_u64(id),
                    kind,
                    owner: Some(principal),
                    parent: parent.map(ObjectId::from_u64),
                    revision,
                    active,
                })
                .unwrap();
        }
        snapshot
            .bind(Binding {
                principal,
                object: ObjectId::from_u64(11),
                rights: 1,
                active: true,
            })
            .unwrap();
        snapshot
            .bind(Binding {
                principal,
                object: ObjectId::from_u64(21),
                rights: 1,
                active: true,
            })
            .unwrap();

        let firewall = snapshot
            .simulate(PolicyChange::Firewall(FirewallChange {
                rule: ObjectId::from_u64(10),
                applies_to: ObjectId::from_u64(11),
                before_revision: 1,
                after_revision: 2,
            }))
            .unwrap();
        let membership = snapshot
            .simulate(PolicyChange::ClusterMembership(ClusterMembershipChange {
                cluster: ObjectId::from_u64(20),
                member: ObjectId::from_u64(21),
                before_active: true,
                after_active: false,
            }))
            .unwrap();
        let rollout = snapshot
            .simulate(PolicyChange::UpdateRollout(UpdateRolloutChange {
                update: ObjectId::from_u64(30),
                from_revision: 1,
                to_revision: 2,
            }))
            .unwrap();

        assert!(firewall.changed && firewall.is_read_only());
        assert!(membership.changed && membership.is_read_only());
        assert!(rollout.changed && rollout.is_read_only());
        assert_eq!(membership.principal_count(), 1);
        assert!(rollout.object_count() >= 4);
    }
}
