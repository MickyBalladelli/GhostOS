#![no_std]

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

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PolicySnapshot<
    const PRINCIPALS: usize = MAX_POLICY_PRINCIPALS,
    const OBJECTS: usize = MAX_POLICY_OBJECTS,
    const BINDINGS: usize = MAX_POLICY_BINDINGS,
> {
    epoch: u64,
    principals: [PrincipalSlot; PRINCIPALS],
    objects: [ObjectSlot; OBJECTS],
    bindings: [BindingSlot; BINDINGS],
}

impl<const PRINCIPALS: usize, const OBJECTS: usize, const BINDINGS: usize>
    PolicySnapshot<PRINCIPALS, OBJECTS, BINDINGS>
{
    pub const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            principals: [PrincipalSlot::EMPTY; PRINCIPALS],
            objects: [ObjectSlot::EMPTY; OBJECTS],
            bindings: [BindingSlot::EMPTY; BINDINGS],
        }
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn add_principal(&mut self, principal: PrincipalId) -> Result<(), SimulationError> {
        // C operates only on this bounded, caller-owned slot array.
        policy_result(unsafe {
            ghostos_policy_add_principal(self.principals.as_mut_ptr(), PRINCIPALS, principal)
        })
    }

    pub fn add_object(&mut self, object: ObjectRecord) -> Result<(), SimulationError> {
        policy_result(unsafe {
            ghostos_policy_add_object(self.objects.as_mut_ptr(), OBJECTS, ObjectSlot::from_record(object))
        })
    }

    pub fn bind(&mut self, binding: Binding) -> Result<(), SimulationError> {
        let mut view = self.view();
        // bind reads only principal/object arrays; the binding array is passed
        // separately for mutation, without a second pointer alias in the view.
        view.bindings = core::ptr::null();
        view.binding_capacity = 0;
        policy_result(unsafe {
            ghostos_policy_bind(&view, self.bindings.as_mut_ptr(), BINDINGS,
                BindingSlot::from_record(binding))
        })
    }

    pub fn simulate(&self, change: PolicyChange) -> Result<SimulationReport, SimulationError> {
        let view = self.view();
        let native_change = NativeChange::from_change(change);
        let mut native = NativeReport::EMPTY;
        // The C snapshot view contains read-only pointers. C writes only the
        // supplied report and retains no pointers into this movable snapshot.
        policy_result(unsafe { ghostos_policy_simulate(&view, &native_change, &mut native) })?;
        let mut report = SimulationReport {
            change,
            before_epoch: native.before_epoch,
            after_epoch: native.after_epoch,
            before_fingerprint: native.before_fingerprint,
            after_fingerprint: native.after_fingerprint,
            changed: native.changed,
            principals: [None; MAX_SIMULATION_AFFECTED],
            principal_count: native.principal_count,
            objects: [None; MAX_SIMULATION_AFFECTED],
            object_count: native.object_count,
        };
        for index in 0..native.principal_count {
            let entry = native.principals[index];
            report.principals[index] = Some(AffectedPrincipal {
                id: entry.id, reason: change_kind(entry.reason),
            });
        }
        for index in 0..native.object_count {
            let entry = native.objects[index];
            report.objects[index] = Some(AffectedObject {
                id: entry.id, kind: object_kind(entry.kind), reason: change_kind(entry.reason),
            });
        }
        Ok(report)
    }

    #[cfg(test)]
    fn binding(&self, principal: PrincipalId, object: ObjectId) -> Option<Binding> {
        let mut binding = BindingSlot::EMPTY;
        if unsafe { ghostos_policy_find_binding(&self.view(), principal, object, &mut binding) } {
            binding.record()
        } else {
            None
        }
    }

    fn view(&self) -> SnapshotView {
        SnapshotView {
            epoch: self.epoch,
            principals: self.principals.as_ptr(), principal_capacity: PRINCIPALS,
            objects: self.objects.as_ptr(), object_capacity: OBJECTS,
            bindings: self.bindings.as_ptr(), binding_capacity: BINDINGS,
        }
    }
}

// Preserve the original Option-record debug view rather than exposing C slots.
impl<const P: usize, const O: usize, const B: usize> core::fmt::Debug for PolicySnapshot<P, O, B> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("PolicySnapshot")
            .field("epoch", &self.epoch)
            .field("principals", &RecordDebug(self.principals.iter().copied().map(PrincipalSlot::record)))
            .field("objects", &RecordDebug(self.objects.iter().copied().map(ObjectSlot::record)))
            .field("bindings", &RecordDebug(self.bindings.iter().copied().map(BindingSlot::record)))
            .finish()
    }
}

struct RecordDebug<I>(I);

impl<I> core::fmt::Debug for RecordDebug<I>
where
    I: Iterator + Clone,
    I::Item: core::fmt::Debug,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.0.clone()).finish()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
struct PrincipalSlot {
    id: PrincipalId,
    active: bool,
    present: bool,
}

impl PrincipalSlot {
    const EMPTY: Self = Self { id: PrincipalId::from_u64(0), active: false, present: false };

    fn record(self) -> Option<PrincipalRecord> {
        self.present.then_some(PrincipalRecord { id: self.id, active: self.active })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
struct ObjectSlot {
    id: ObjectId,
    owner: PrincipalId,
    parent: ObjectId,
    revision: u64,
    kind: u8,
    active: bool,
    has_owner: bool,
    has_parent: bool,
    present: bool,
}

impl ObjectSlot {
    const EMPTY: Self = Self {
        id: ObjectId::from_u64(0), owner: PrincipalId::from_u64(0), parent: ObjectId::from_u64(0),
        revision: 0, kind: 0, active: false, has_owner: false, has_parent: false, present: false,
    };

    fn from_record(record: ObjectRecord) -> Self {
        Self {
            id: record.id, owner: record.owner.unwrap_or(PrincipalId::from_u64(0)),
            parent: record.parent.unwrap_or(ObjectId::from_u64(0)), revision: record.revision,
            kind: record.kind as u8, active: record.active, has_owner: record.owner.is_some(),
            has_parent: record.parent.is_some(), present: true,
        }
    }

    fn record(self) -> Option<ObjectRecord> {
        if !self.present { return None }
        Some(ObjectRecord {
            id: self.id, kind: object_kind(self.kind), revision: self.revision, active: self.active,
            owner: self.has_owner.then_some(self.owner), parent: self.has_parent.then_some(self.parent),
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
struct BindingSlot {
    principal: PrincipalId,
    object: ObjectId,
    rights: u64,
    active: bool,
    present: bool,
}

impl BindingSlot {
    const EMPTY: Self = Self { principal: PrincipalId::from_u64(0), object: ObjectId::from_u64(0),
        rights: 0, active: false, present: false };

    fn from_record(record: Binding) -> Self {
        Self { principal: record.principal, object: record.object, rights: record.rights,
            active: record.active, present: true }
    }

    fn record(self) -> Option<Binding> {
        self.present.then_some(Binding { principal: self.principal, object: self.object,
            rights: self.rights, active: self.active })
    }
}

#[repr(C)]
struct SnapshotView {
    epoch: u64,
    principals: *const PrincipalSlot,
    principal_capacity: usize,
    objects: *const ObjectSlot,
    object_capacity: usize,
    bindings: *const BindingSlot,
    binding_capacity: usize,
}

#[repr(C)]
struct NativeChange {
    principal: PrincipalId,
    object: ObjectId,
    related: ObjectId,
    before: u64,
    after: u64,
    kind: u8,
    before_active: bool,
    after_active: bool,
}

impl NativeChange {
    fn from_change(change: PolicyChange) -> Self {
        let mut native = Self { principal: PrincipalId::from_u64(0), object: ObjectId::from_u64(0),
            related: ObjectId::from_u64(0), before: 0, after: 0, kind: change.kind() as u8,
            before_active: false, after_active: false };
        match change {
            PolicyChange::Capability(value) => {
                native.principal = value.principal;
                native.object = value.object;
                native.before = value.before_rights;
                native.after = value.after_rights;
            }
            PolicyChange::Firewall(value) => {
                native.object = value.rule;
                native.related = value.applies_to;
                native.before = value.before_revision;
                native.after = value.after_revision;
            }
            PolicyChange::PackageActivation(value) => {
                native.object = value.package;
                native.before = value.before_revision;
                native.after = value.after_revision;
                native.after_active = value.activate;
            }
            PolicyChange::ClusterMembership(value) => {
                native.object = value.cluster;
                native.related = value.member;
                native.before_active = value.before_active;
                native.after_active = value.after_active;
            }
            PolicyChange::UpdateRollout(value) => {
                native.object = value.update;
                native.before = value.from_revision;
                native.after = value.to_revision;
            }
        }
        native
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeAffectedPrincipal { id: PrincipalId, reason: u8 }

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeAffectedObject { id: ObjectId, kind: u8, reason: u8 }

#[repr(C)]
struct NativeReport {
    before_epoch: u64,
    after_epoch: u64,
    before_fingerprint: u64,
    after_fingerprint: u64,
    changed: bool,
    principals: [NativeAffectedPrincipal; MAX_SIMULATION_AFFECTED],
    principal_count: usize,
    objects: [NativeAffectedObject; MAX_SIMULATION_AFFECTED],
    object_count: usize,
}

impl NativeReport {
    const EMPTY: Self = Self {
        before_epoch: 0, after_epoch: 0, before_fingerprint: 0, after_fingerprint: 0, changed: false,
        principals: [NativeAffectedPrincipal { id: PrincipalId::from_u64(0), reason: 0 }; MAX_SIMULATION_AFFECTED],
        principal_count: 0,
        objects: [NativeAffectedObject { id: ObjectId::from_u64(0), kind: 0, reason: 0 }; MAX_SIMULATION_AFFECTED],
        object_count: 0,
    };
}

fn object_kind(kind: u8) -> ObjectKind {
    match kind {
        1 => ObjectKind::Capability,
        2 => ObjectKind::FirewallRule,
        3 => ObjectKind::Network,
        4 => ObjectKind::Package,
        5 => ObjectKind::PackageProcess,
        6 => ObjectKind::Cluster,
        7 => ObjectKind::ClusterMember,
        8 => ObjectKind::Update,
        9 => ObjectKind::System,
        _ => panic!("invalid C policy object kind"),
    }
}

fn change_kind(kind: u8) -> ChangeKind {
    match kind {
        0 => ChangeKind::Capability,
        1 => ChangeKind::Firewall,
        2 => ChangeKind::PackageActivation,
        3 => ChangeKind::ClusterMembership,
        4 => ChangeKind::UpdateRollout,
        _ => panic!("invalid C policy change kind"),
    }
}

fn policy_result(code: u32) -> Result<(), SimulationError> {
    Err(match code {
        0 => return Ok(()),
        1 => SimulationError::Capacity,
        2 => SimulationError::UnknownPrincipal,
        3 => SimulationError::UnknownObject,
        4 => SimulationError::InvalidObjectKind,
        5 => SimulationError::InvalidChange,
        6 => SimulationError::StaleSnapshot,
        7 => SimulationError::TooManyAffected,
        _ => panic!("invalid C policy result"),
    })
}

const _: () = {
    assert!(core::mem::size_of::<PrincipalId>() == 32);
    assert!(core::mem::size_of::<ObjectId>() == 32);
    assert!(core::mem::size_of::<PrincipalSlot>() == 34);
    assert!(core::mem::offset_of!(PrincipalSlot, id) == 0);
    assert!(core::mem::offset_of!(PrincipalSlot, active) == 32);
    assert!(core::mem::offset_of!(PrincipalSlot, present) == 33);
    assert!(core::mem::size_of::<ObjectSlot>() == 112);
    assert!(core::mem::offset_of!(ObjectSlot, id) == 0);
    assert!(core::mem::offset_of!(ObjectSlot, owner) == 32);
    assert!(core::mem::offset_of!(ObjectSlot, parent) == 64);
    assert!(core::mem::offset_of!(ObjectSlot, revision) == 96);
    assert!(core::mem::offset_of!(ObjectSlot, kind) == 104);
    assert!(core::mem::offset_of!(ObjectSlot, active) == 105);
    assert!(core::mem::offset_of!(ObjectSlot, has_owner) == 106);
    assert!(core::mem::offset_of!(ObjectSlot, has_parent) == 107);
    assert!(core::mem::offset_of!(ObjectSlot, present) == 108);
    assert!(core::mem::size_of::<BindingSlot>() == 80);
    assert!(core::mem::offset_of!(BindingSlot, principal) == 0);
    assert!(core::mem::offset_of!(BindingSlot, object) == 32);
    assert!(core::mem::offset_of!(BindingSlot, rights) == 64);
    assert!(core::mem::offset_of!(BindingSlot, active) == 72);
    assert!(core::mem::offset_of!(BindingSlot, present) == 73);
    assert!(core::mem::size_of::<SnapshotView>() == 56);
    assert!(core::mem::offset_of!(SnapshotView, epoch) == 0);
    assert!(core::mem::offset_of!(SnapshotView, principals) == 8);
    assert!(core::mem::offset_of!(SnapshotView, principal_capacity) == 16);
    assert!(core::mem::offset_of!(SnapshotView, objects) == 24);
    assert!(core::mem::offset_of!(SnapshotView, object_capacity) == 32);
    assert!(core::mem::offset_of!(SnapshotView, bindings) == 40);
    assert!(core::mem::offset_of!(SnapshotView, binding_capacity) == 48);
    assert!(core::mem::size_of::<NativeChange>() == 120);
    assert!(core::mem::offset_of!(NativeChange, principal) == 0);
    assert!(core::mem::offset_of!(NativeChange, object) == 32);
    assert!(core::mem::offset_of!(NativeChange, related) == 64);
    assert!(core::mem::offset_of!(NativeChange, before) == 96);
    assert!(core::mem::offset_of!(NativeChange, after) == 104);
    assert!(core::mem::offset_of!(NativeChange, kind) == 112);
    assert!(core::mem::offset_of!(NativeChange, before_active) == 113);
    assert!(core::mem::offset_of!(NativeChange, after_active) == 114);
    assert!(core::mem::size_of::<NativeAffectedPrincipal>() == 33);
    assert!(core::mem::offset_of!(NativeAffectedPrincipal, id) == 0);
    assert!(core::mem::offset_of!(NativeAffectedPrincipal, reason) == 32);
    assert!(core::mem::size_of::<NativeAffectedObject>() == 34);
    assert!(core::mem::offset_of!(NativeAffectedObject, id) == 0);
    assert!(core::mem::offset_of!(NativeAffectedObject, kind) == 32);
    assert!(core::mem::offset_of!(NativeAffectedObject, reason) == 33);
    assert!(core::mem::size_of::<NativeReport>() == 8632);
    assert!(core::mem::offset_of!(NativeReport, before_epoch) == 0);
    assert!(core::mem::offset_of!(NativeReport, after_epoch) == 8);
    assert!(core::mem::offset_of!(NativeReport, before_fingerprint) == 16);
    assert!(core::mem::offset_of!(NativeReport, after_fingerprint) == 24);
    assert!(core::mem::offset_of!(NativeReport, changed) == 32);
    assert!(core::mem::offset_of!(NativeReport, principals) == 33);
    assert!(core::mem::offset_of!(NativeReport, principal_count) == 4264);
    assert!(core::mem::offset_of!(NativeReport, objects) == 4272);
    assert!(core::mem::offset_of!(NativeReport, object_count) == 8624);
};

unsafe extern "C" {
    fn ghostos_policy_add_principal(slots: *mut PrincipalSlot, capacity: usize, id: PrincipalId) -> u32;
    fn ghostos_policy_add_object(slots: *mut ObjectSlot, capacity: usize, object: ObjectSlot) -> u32;
    fn ghostos_policy_bind(snapshot: *const SnapshotView, slots: *mut BindingSlot,
        capacity: usize, binding: BindingSlot) -> u32;
    fn ghostos_policy_simulate(snapshot: *const SnapshotView, change: *const NativeChange,
        report: *mut NativeReport) -> u32;
    #[cfg(test)]
    fn ghostos_policy_find_binding(snapshot: *const SnapshotView, principal: PrincipalId,
        object: ObjectId, out: *mut BindingSlot) -> bool;
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
