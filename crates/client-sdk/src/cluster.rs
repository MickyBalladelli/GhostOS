use ghostos_fabric::NodeId;

use crate::ProtocolError;

pub const MAX_CLUSTER_NAME_BYTES: usize = 64;
pub const MAX_CLUSTER_MEMBERS: usize = 64;
pub const MAX_CLUSTER_INVITATIONS: usize = 32;
pub const MAX_CLUSTER_AUDIT_EVENTS: usize = 32;
pub const MAX_CLUSTER_CHANGES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ClusterId(u128);

impl ClusterId {
    pub const fn new(raw: u128) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u128 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundedText<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    length: u8,
}

impl<const CAPACITY: usize> BoundedText<CAPACITY> {
    pub fn new(value: &str) -> Result<Self, ProtocolError> {
        if value.is_empty() || value.len() > CAPACITY || !value.is_ascii() {
            return Err(ProtocolError::InvalidValue);
        }
        let mut bytes = [0; CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self { bytes, length: value.len() as u8 })
    }

    pub(crate) fn from_wire(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let length = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
        if length == 0 || length > CAPACITY || !bytes[..length].is_ascii() {
            return Err(ProtocolError::InvalidValue);
        }
        let mut value = [0; CAPACITY];
        value[..length].copy_from_slice(&bytes[..length]);
        if bytes[length..].iter().any(|byte| *byte != 0) {
            return Err(ProtocolError::InvalidValue);
        }
        Ok(Self { bytes: value, length: length as u8 })
    }

    pub const fn as_bytes(self) -> [u8; CAPACITY] {
        self.bytes
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize]).unwrap_or("")
    }
}

pub type ClusterName = BoundedText<MAX_CLUSTER_NAME_BYTES>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterLifecycle {
    Creating = 1,
    PendingAdmission = 2,
    Active = 3,
    Degraded = 4,
    Partitioned = 5,
    Draining = 6,
    Leaving = 7,
    Retired = 8,
    Deleted = 9,
}

impl ClusterLifecycle {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Creating),
            2 => Ok(Self::PendingAdmission),
            3 => Ok(Self::Active),
            4 => Ok(Self::Degraded),
            5 => Ok(Self::Partitioned),
            6 => Ok(Self::Draining),
            7 => Ok(Self::Leaving),
            8 => Ok(Self::Retired),
            9 => Ok(Self::Deleted),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterHealth {
    Healthy = 1,
    Degraded = 2,
    Partitioned = 3,
    Unavailable = 4,
}

impl ClusterHealth {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Healthy),
            2 => Ok(Self::Degraded),
            3 => Ok(Self::Partitioned),
            4 => Ok(Self::Unavailable),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MemberState {
    Pending = 1,
    Approved = 2,
    Joined = 3,
    Draining = 4,
    Left = 5,
    Fenced = 6,
    Expelled = 7,
}

impl MemberState {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Pending),
            2 => Ok(Self::Approved),
            3 => Ok(Self::Joined),
            4 => Ok(Self::Draining),
            5 => Ok(Self::Left),
            6 => Ok(Self::Fenced),
            7 => Ok(Self::Expelled),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MemberRole {
    Voter = 1,
    NonVoter = 2,
    Witness = 3,
}

impl MemberRole {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Voter),
            2 => Ok(Self::NonVoter),
            3 => Ok(Self::Witness),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSummary {
    pub cluster: ClusterId,
    pub name: ClusterName,
    pub lifecycle: ClusterLifecycle,
    pub health: ClusterHealth,
    pub generation: u64,
    pub sampled_at_us: u64,
    pub leader: NodeId,
    pub coordinator: NodeId,
    pub member_count: u16,
    pub healthy_members: u16,
    pub voting_members: u16,
    pub quorum_required: u16,
    pub quorum_available: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMember {
    pub node: NodeId,
    pub state: MemberState,
    pub role: MemberRole,
    pub health: crate::NodeHealth,
    pub last_seen_us: u64,
    pub cpu_load_permille: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberList {
    pub generation: u64,
    members: [Option<ClusterMember>; MAX_CLUSTER_MEMBERS],
}

impl MemberList {
    pub const fn new(generation: u64) -> Self {
        Self { generation, members: [None; MAX_CLUSTER_MEMBERS] }
    }

    pub fn push(&mut self, member: ClusterMember) -> Result<(), ProtocolError> {
        if self.members().any(|entry| entry.node == member.node) {
            return Err(ProtocolError::DuplicateNode);
        }
        let slot = self.members.iter_mut().find(|entry| entry.is_none()).ok_or(ProtocolError::Capacity)?;
        *slot = Some(member);
        Ok(())
    }

    pub fn members(&self) -> impl Iterator<Item = ClusterMember> + '_ {
        self.members.iter().flatten().copied()
    }

    pub fn member_count(&self) -> usize {
        self.members().count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InvitationState {
    Pending = 1,
    Approved = 2,
    Used = 3,
    Rejected = 4,
    Revoked = 5,
    Expired = 6,
}

impl InvitationState {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Pending),
            2 => Ok(Self::Approved),
            3 => Ok(Self::Used),
            4 => Ok(Self::Rejected),
            5 => Ok(Self::Revoked),
            6 => Ok(Self::Expired),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterInvitation {
    pub invitation: u64,
    pub node: NodeId,
    pub state: InvitationState,
    pub issued_at_us: u64,
    pub expires_at_us: u64,
    pub scope: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvitationList {
    pub generation: u64,
    invitations: [Option<ClusterInvitation>; MAX_CLUSTER_INVITATIONS],
}

impl InvitationList {
    pub const fn new(generation: u64) -> Self {
        Self { generation, invitations: [None; MAX_CLUSTER_INVITATIONS] }
    }

    pub fn push(&mut self, invitation: ClusterInvitation) -> Result<(), ProtocolError> {
        if self.invitations().any(|entry| entry.invitation == invitation.invitation) {
            return Err(ProtocolError::InvalidValue);
        }
        let slot = self.invitations.iter_mut().find(|entry| entry.is_none()).ok_or(ProtocolError::Capacity)?;
        *slot = Some(invitation);
        Ok(())
    }

    pub fn invitations(&self) -> impl Iterator<Item = ClusterInvitation> + '_ {
        self.invitations.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinPlan {
    pub generation: u64,
    pub invitation: u64,
    pub node: NodeId,
    pub expires_at_us: u64,
    pub requires_approval: bool,
    pub steps: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeavePlan {
    pub generation: u64,
    pub node: NodeId,
    pub workload_count: u32,
    pub lease_count: u32,
    pub drain_required: bool,
    pub force_allowed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterHealthSnapshot {
    pub generation: u64,
    pub health: ClusterHealth,
    pub quorum: bool,
    pub heartbeat_period_us: u64,
    pub missed_heartbeat_limit: u16,
    pub last_change_us: u64,
    pub healthy_nodes: u16,
    pub degraded_nodes: u16,
    pub failed_nodes: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct ClusterResources {
    pub generation: u64,
    pub cpu_capacity: u64,
    pub cpu_available: u64,
    pub memory_capacity_bytes: u64,
    pub memory_available_bytes: u64,
    pub cxl_capacity_bytes: u64,
    pub cxl_available_bytes: u64,
    pub storage_capacity_bytes: u64,
    pub storage_available_bytes: u64,
    pub network_bandwidth_mbps: u64,
    pub accelerator_capacity: u64,
    pub accelerator_available: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterAuditEvent {
    pub sequence: u64,
    pub timestamp_us: u64,
    pub correlation: u128,
    pub actor: NodeId,
    pub operation: u16,
    pub status: u16,
    pub target: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditEventList {
    pub generation: u64,
    events: [Option<ClusterAuditEvent>; MAX_CLUSTER_AUDIT_EVENTS],
}

impl AuditEventList {
    pub const fn new(generation: u64) -> Self {
        Self { generation, events: [None; MAX_CLUSTER_AUDIT_EVENTS] }
    }

    pub fn push(&mut self, event: ClusterAuditEvent) -> Result<(), ProtocolError> {
        let slot = self.events.iter_mut().find(|entry| entry.is_none()).ok_or(ProtocolError::Capacity)?;
        *slot = Some(event);
        Ok(())
    }

    pub fn events(&self) -> impl Iterator<Item = ClusterAuditEvent> + '_ {
        self.events.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCreateRequest {
    pub cluster: ClusterId,
    pub name: ClusterName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterJoinRequest {
    pub cluster: ClusterId,
    pub invitation: u64,
    pub node: NodeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterLeaveRequest {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterRemoveRequest {
    pub cluster: ClusterId,
    pub confirmation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleReceipt {
    pub operation: u64,
    pub generation: u64,
    pub lifecycle: ClusterLifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SubscriptionKind {
    Membership = 1,
    Health = 2,
    Topology = 3,
    Resources = 4,
    Lifecycle = 5,
}

impl SubscriptionKind {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Membership),
            2 => Ok(Self::Health),
            3 => Ok(Self::Topology),
            4 => Ok(Self::Resources),
            5 => Ok(Self::Lifecycle),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Subscription {
    pub kind: SubscriptionKind,
    pub cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeEvent {
    pub sequence: u64,
    pub generation: u64,
    pub kind: SubscriptionKind,
    pub operation: u16,
    pub timestamp_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeBatch {
    pub subscription: Subscription,
    pub events: [Option<ChangeEvent>; MAX_CLUSTER_CHANGES],
    pub next_cursor: u64,
    pub has_more: bool,
}

impl ChangeBatch {
    pub const fn empty(subscription: Subscription) -> Self {
        Self { subscription, events: [None; MAX_CLUSTER_CHANGES], next_cursor: subscription.cursor, has_more: false }
    }

    pub fn events(&self) -> impl Iterator<Item = ChangeEvent> + '_ {
        self.events.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeLogError {
    Capacity,
    Invalid,
}

/// Fixed-size server-side change log for cursor subscriptions. Publishing
/// never allocates; when full, the oldest change is overwritten.
pub struct ChangeLog<const CAPACITY: usize = MAX_CLUSTER_CHANGES> {
    events: [Option<ChangeEvent>; CAPACITY],
    head: usize,
    length: usize,
    next_sequence: u64,
}

impl<const CAPACITY: usize> ChangeLog<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1 && CAPACITY <= MAX_CLUSTER_CHANGES);
        Self { events: [None; CAPACITY], head: 0, length: 0, next_sequence: 1 }
    }

    pub fn publish(
        &mut self,
        generation: u64,
        kind: SubscriptionKind,
        operation: u16,
        timestamp_us: u64,
    ) -> Result<u64, ChangeLogError> {
        if generation == 0 || operation == 0 || timestamp_us == 0 {
            return Err(ChangeLogError::Invalid);
        }
        let event = ChangeEvent {
            sequence: self.next_sequence,
            generation,
            kind,
            operation,
            timestamp_us,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        let index = (self.head + self.length) % CAPACITY;
        if self.length == CAPACITY {
            self.events[self.head] = Some(event);
            self.head = (self.head + 1) % CAPACITY;
        } else {
            self.events[index] = Some(event);
            self.length += 1;
        }
        Ok(event.sequence)
    }

    pub fn poll(
        &self,
        subscription: Subscription,
        limit: u8,
    ) -> Result<ChangeBatch, ChangeLogError> {
        if limit == 0 || limit as usize > CAPACITY {
            return Err(ChangeLogError::Capacity);
        }
        let mut batch = ChangeBatch::empty(subscription);
        let mut matched = 0;
        for offset in 0..self.length {
            let index = (self.head + offset) % CAPACITY;
            let Some(event) = self.events[index] else { continue };
            if event.sequence <= subscription.cursor || event.kind != subscription.kind {
                continue;
            }
            if matched < limit as usize {
                batch.events[matched] = Some(event);
                matched += 1;
                batch.next_cursor = event.sequence;
            } else {
                batch.has_more = true;
                break
            }
        }
        Ok(batch)
    }
}

impl<const CAPACITY: usize> Default for ChangeLog<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
