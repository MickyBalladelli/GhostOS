use crate::{AdmissionEndpoint, ClusterId, QuorumPolicy};
use synos_admission::{AdmissionController, AdmissionOutcome, AdmissionPriority, WorkClass};
use synos_fabric::NodeId;
use synos_policy::{
    ClusterMembershipChange, ObjectId, PolicyChange, PolicySnapshot, SimulationError,
    SimulationReport,
};
use synos_status::{IntoStatus, Status};

pub const MAX_MEMBERSHIP_REGISTRY: usize = 64;
pub const MAX_MEMBERSHIP_LABEL_BYTES: usize = 32;
pub const MAX_CONSENSUS_LOG: usize = 64;
pub const MEMBERSHIP_STATE_FILE: &str = "/system/cluster/membership.dat";
pub const MEMBERSHIP_MAGIC: &[u8; 8] = b"SYNMEMB1";
pub const MEMBERSHIP_FORMAT_VERSION: u16 = 1;
pub const MEMBERSHIP_HEADER_BYTES: usize = 80;
pub const MEMBERSHIP_RECORD_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipError {
    Admission(AdmissionOutcome),
    InvalidConfiguration,
    InvalidLabel,
    InvalidState,
    Capacity,
    DuplicateIdentity,
    UnknownNode,
    WrongCluster,
    StaleAdvertisement,
    Replay,
    QuorumUnavailable,
    NotLeader,
    NoLeader,
    SplitBrain,
    ProposalPending,
    ProposalNotFound,
    AlreadyAcknowledged,
    InvalidChange,
    BufferTooSmall { required: usize },
    Corrupt,
    Persistence,
    ConsumerRejected,
}

impl IntoStatus for MembershipError {
    fn status(self) -> Status {
        match self {
            Self::Admission(outcome) => match outcome.action {
                synos_admission::AdmissionAction::Dropped => Status::NO_SPACE,
                synos_admission::AdmissionAction::Delayed
                | synos_admission::AdmissionAction::Retried => Status::BUSY,
                synos_admission::AdmissionAction::Admitted => Status::INTERNAL,
            },
            Self::Capacity | Self::BufferTooSmall { .. } => Status::NO_SPACE,
            Self::UnknownNode | Self::ProposalNotFound => Status::NOT_FOUND,
            Self::DuplicateIdentity
            | Self::StaleAdvertisement
            | Self::Replay
            | Self::SplitBrain
            | Self::ProposalPending
            | Self::AlreadyAcknowledged
            | Self::InvalidChange => Status::CONFLICT,
            Self::QuorumUnavailable
            | Self::NoLeader
            | Self::NotLeader
            | Self::Persistence
            | Self::ConsumerRejected => Status::BUSY,
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidConfiguration
            | Self::InvalidLabel
            | Self::InvalidState
            | Self::WrongCluster => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MembershipLabel {
    bytes: [u8; MAX_MEMBERSHIP_LABEL_BYTES],
    len: u8,
}

impl MembershipLabel {
    pub const EMPTY: Self = Self {
        bytes: [0; MAX_MEMBERSHIP_LABEL_BYTES],
        len: 0,
    };

    pub fn new(value: &str) -> Result<Self, MembershipError> {
        if value.len() > MAX_MEMBERSHIP_LABEL_BYTES
            || value.bytes().any(|byte| byte == 0 || byte.is_ascii_whitespace())
        {
            return Err(MembershipError::InvalidLabel)
        }
        let mut bytes = [0; MAX_MEMBERSHIP_LABEL_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MemberRole {
    Voter = 1,
    Witness = 2,
    Observer = 3,
}

impl MemberRole {
    pub const fn is_voting(self) -> bool {
        matches!(self, Self::Voter | Self::Witness)
    }

    fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Voter,
            2 => Self::Witness,
            3 => Self::Observer,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MemberHealth {
    Joining = 1,
    Healthy = 2,
    Suspect = 3,
    Failed = 4,
    Fenced = 5,
    Left = 6,
}

impl MemberHealth {
    pub const fn can_vote(self) -> bool {
        matches!(self, Self::Healthy | Self::Suspect)
    }

    fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Joining,
            2 => Self::Healthy,
            3 => Self::Suspect,
            4 => Self::Failed,
            5 => Self::Fenced,
            6 => Self::Left,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryMember {
    pub node: NodeId,
    pub role: MemberRole,
    pub health: MemberHealth,
    pub reachable: bool,
    pub fingerprint: [u8; 32],
    pub endpoint: AdmissionEndpoint,
    pub capacity: u64,
    pub zone: MembershipLabel,
    pub rack: MembershipLabel,
    pub generation: u64,
    pub last_seen_generation: u64,
    pub membership_epoch: u64,
    pub last_seen_us: u64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberSpec {
    pub node: NodeId,
    pub role: MemberRole,
    pub fingerprint: [u8; 32],
    pub endpoint: AdmissionEndpoint,
    pub capacity: u64,
    pub zone: MembershipLabel,
    pub rack: MembershipLabel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MembershipAdvertisement {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub role: MemberRole,
    pub fingerprint: [u8; 32],
    pub endpoint: AdmissionEndpoint,
    pub capacity: u64,
    pub zone: MembershipLabel,
    pub rack: MembershipLabel,
    pub membership_epoch: u64,
    pub generation: u64,
    pub sequence: u64,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumView {
    pub voting_members: u16,
    pub available_votes: u16,
    pub required_votes: u16,
    pub witnesses: u16,
    pub has_quorum: bool,
    pub degraded: bool,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElectionResult {
    pub term: u64,
    pub leader: NodeId,
    pub votes: u16,
    pub required_votes: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipOperation {
    Add(MemberSpec),
    Remove(NodeId),
    ChangeRole { node: NodeId, role: MemberRole },
    Fence(NodeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusProposal {
    pub index: u64,
    pub term: u64,
    pub epoch: u64,
    pub acknowledgements: u16,
    pub required_votes: u16,
    pub committed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusCommit {
    pub index: u64,
    pub term: u64,
    pub epoch: u64,
    pub acknowledgements: u16,
    pub required_votes: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingProposal {
    index: u64,
    term: u64,
    epoch: u64,
    operation: MembershipOperation,
    acknowledgements: [Option<NodeId>; MAX_MEMBERSHIP_REGISTRY],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommittedOperation {
    index: u64,
    term: u64,
    epoch: u64,
    operation: MembershipOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MembershipSnapshot {
    pub cluster: ClusterId,
    pub epoch: u64,
    pub generation: u64,
    pub term: u64,
    pub leader: Option<NodeId>,
    pub quorum: QuorumView,
    pub commit_index: u64,
}

pub struct MembershipRegistry<const MEMBERS: usize = MAX_MEMBERSHIP_REGISTRY> {
    pub cluster: ClusterId,
    pub quorum_policy: QuorumPolicy,
    pub epoch: u64,
    pub generation: u64,
    pub term: u64,
    pub leader: Option<NodeId>,
    voted_for: Option<NodeId>,
    commit_index: u64,
    next_index: u64,
    members: [Option<RegistryMember>; MEMBERS],
    pending: Option<PendingProposal>,
    committed: [Option<CommittedOperation>; MAX_CONSENSUS_LOG],
}

impl<const MEMBERS: usize> MembershipRegistry<MEMBERS> {
    pub fn new(cluster: ClusterId, quorum_policy: QuorumPolicy) -> Result<Self, MembershipError> {
        if quorum_policy.voting_members == 0
            || quorum_policy.required_votes == 0
            || quorum_policy.required_votes > quorum_policy.voting_members
            || MEMBERS == 0
        {
            return Err(MembershipError::InvalidConfiguration)
        }
        Ok(Self {
            cluster,
            quorum_policy,
            epoch: 1,
            generation: 1,
            term: 0,
            leader: None,
            voted_for: None,
            commit_index: 0,
            next_index: 1,
            members: [None; MEMBERS],
            pending: None,
            committed: [None; MAX_CONSENSUS_LOG],
        })
    }

    pub fn snapshot(&self) -> MembershipSnapshot {
        MembershipSnapshot {
            cluster: self.cluster,
            epoch: self.epoch,
            generation: self.generation,
            term: self.term,
            leader: self.leader,
            quorum: self.quorum(),
            commit_index: self.commit_index,
        }
    }

    pub fn snapshot_with_admission<const CAPACITY: usize>(
        &self,
        admission: &mut AdmissionController<CAPACITY>,
        priority: AdmissionPriority,
    ) -> Result<MembershipSnapshot, MembershipError> {
        let outcome = admission.admit(WorkClass::Snapshot, priority);
        let Some(lease) = outcome.lease() else {
            return Err(MembershipError::Admission(outcome))
        };
        let snapshot = self.snapshot();
        let _ = admission.finish(lease);
        Ok(snapshot)
    }

    /// Preview a membership transition without proposing, acknowledging, or
    /// committing a consensus operation.
    pub fn simulate_membership_policy<
        const PRINCIPALS: usize,
        const OBJECTS: usize,
        const BINDINGS: usize,
    >(
        &self,
        policy: &PolicySnapshot<PRINCIPALS, OBJECTS, BINDINGS>,
        cluster: ObjectId,
        member: NodeId,
        after_active: bool,
    ) -> Result<SimulationReport, SimulationError> {
        let before_active = self.member(member).is_some();
        let member = ObjectId::from_u64(member.raw() as u64);
        policy.simulate(PolicyChange::ClusterMembership(ClusterMembershipChange {
            cluster,
            member,
            before_active,
            after_active,
        }))
    }

    pub fn member(&self, node: NodeId) -> Option<RegistryMember> {
        self.members.iter().flatten().find(|member| member.node == node).copied()
    }

    pub fn members(&self) -> impl Iterator<Item = RegistryMember> + '_ {
        self.members.iter().flatten().copied()
    }

    pub fn committed_index(&self) -> u64 {
        self.commit_index
    }

    pub fn leader(&self) -> Option<NodeId> {
        self.leader
    }

    pub fn coordinator(&self) -> Option<NodeId> {
        self.leader
    }

    pub fn is_read_only(&self) -> bool {
        !self.quorum().has_quorum
    }

    pub fn can_write(&self) -> bool {
        self.quorum().has_quorum
            && self.leader.is_some_and(|leader| {
                self.member(leader)
                    .is_some_and(|member| member.reachable && member.health.can_vote())
            })
    }

    pub fn quorum(&self) -> QuorumView {
        let mut voting_members: u16 = 0;
        let mut available_votes: u16 = 0;
        let mut witnesses: u16 = 0;
        for member in self.members.iter().flatten() {
            if member.role.is_voting() {
                voting_members = voting_members.saturating_add(1);
                if member.role == MemberRole::Witness {
                    witnesses = witnesses.saturating_add(1);
                }
                if member.reachable && member.health.can_vote() {
                    available_votes = available_votes.saturating_add(1);
                }
            }
        }
        let has_quorum = available_votes >= self.quorum_policy.required_votes;
        QuorumView {
            voting_members,
            available_votes,
            required_votes: self.quorum_policy.required_votes,
            witnesses,
            has_quorum,
            degraded: available_votes < voting_members,
            read_only: !has_quorum,
        }
    }

    pub fn bootstrap_member(
        &mut self,
        spec: MemberSpec,
        now_us: u64,
    ) -> Result<RegistryMember, MembershipError> {
        if self.members.iter().flatten().next().is_some() {
            return Err(MembershipError::InvalidState)
        }
        self.validate_add(spec)?;
        self.insert_member(spec, MemberHealth::Healthy, true, now_us)
    }

    pub fn register_member(&mut self, spec: MemberSpec, _now_us: u64) -> Result<ConsensusProposal, MembershipError> {
        self.ensure_write_ready(self.leader)?;
        self.validate_add(spec)?;
        self.propose_membership_change(self.leader.ok_or(MembershipError::NoLeader)?, MembershipOperation::Add(spec))
    }

    pub fn register_member_with_admission<const CAPACITY: usize>(
        &mut self,
        spec: MemberSpec,
        now_us: u64,
        admission: &mut AdmissionController<CAPACITY>,
        priority: AdmissionPriority,
    ) -> Result<ConsensusProposal, MembershipError> {
        let outcome = admission.admit(WorkClass::MembershipChange, priority);
        let Some(lease) = outcome.lease() else {
            return Err(MembershipError::Admission(outcome))
        };
        let result = self.register_member(spec, now_us);
        let _ = admission.finish(lease);
        result
    }

    pub fn observe_advertisement(
        &mut self,
        advertisement: MembershipAdvertisement,
        now_us: u64,
    ) -> Result<(), MembershipError> {
        if advertisement.cluster != self.cluster {
            return Err(MembershipError::WrongCluster)
        }
        if advertisement.expires_at_us <= now_us {
            return Err(MembershipError::StaleAdvertisement)
        }
        if self.member(advertisement.node).is_none()
            && self.members.iter().flatten().any(|member| member.fingerprint == advertisement.fingerprint)
        {
            return Err(MembershipError::DuplicateIdentity)
        }
        let member = self
            .members
            .iter_mut()
            .flatten()
            .find(|member| member.node == advertisement.node)
            .ok_or(MembershipError::UnknownNode)?;
        if member.fingerprint != advertisement.fingerprint {
            return Err(MembershipError::DuplicateIdentity)
        }
        if advertisement.role != member.role
            || advertisement.membership_epoch != self.epoch
            || advertisement.generation < member.generation
            || advertisement.generation > self.generation
            || advertisement.sequence <= member.sequence
        {
            return Err(MembershipError::StaleAdvertisement)
        }
        member.role = advertisement.role;
        member.endpoint = advertisement.endpoint;
        member.capacity = advertisement.capacity;
        member.zone = advertisement.zone;
        member.rack = advertisement.rack;
        member.last_seen_generation = advertisement.generation;
        member.membership_epoch = advertisement.membership_epoch;
        member.last_seen_us = now_us;
        member.sequence = advertisement.sequence;
        member.reachable = true;
        member.health = match member.health {
            MemberHealth::Fenced | MemberHealth::Left => member.health,
            _ => MemberHealth::Healthy,
        };
        Ok(())
    }

    pub fn set_reachability(
        &mut self,
        node: NodeId,
        reachable: bool,
        now_us: u64,
    ) -> Result<(), MembershipError> {
        let member = self.member_mut(node)?;
        member.reachable = reachable;
        member.last_seen_us = now_us;
        if !reachable && member.health == MemberHealth::Healthy {
            member.health = MemberHealth::Suspect;
        }
        if !reachable && self.leader == Some(node) {
            self.leader = None;
        }
        Ok(())
    }

    pub fn set_health(&mut self, node: NodeId, health: MemberHealth) -> Result<(), MembershipError> {
        let member = self.member_mut(node)?;
        member.health = health;
        if matches!(health, MemberHealth::Failed | MemberHealth::Fenced | MemberHealth::Left) {
            member.reachable = false;
        }
        if self.leader == Some(node) && !health.can_vote() {
            self.leader = None;
        }
        Ok(())
    }

    pub fn elect_leader(&mut self) -> Result<ElectionResult, MembershipError> {
        let quorum = self.quorum();
        if !quorum.has_quorum {
            self.leader = None;
            return Err(MembershipError::QuorumUnavailable)
        }
        let candidate = self
            .members
            .iter()
            .flatten()
            .filter(|member| member.role.is_voting() && member.reachable && member.health.can_vote())
            .map(|member| member.node)
            .min()
            .ok_or(MembershipError::NoLeader)?;
        self.term = self.term.checked_add(1).ok_or(MembershipError::InvalidConfiguration)?;
        self.voted_for = Some(candidate);
        self.leader = Some(candidate);
        Ok(ElectionResult {
            term: self.term,
            leader: candidate,
            votes: quorum.available_votes,
            required_votes: quorum.required_votes,
        })
    }

    pub fn accept_leader_heartbeat(
        &mut self,
        node: NodeId,
        term: u64,
        epoch: u64,
        generation: u64,
    ) -> Result<(), MembershipError> {
        if term == 0 || epoch != self.epoch || generation != self.generation {
            return Err(MembershipError::StaleAdvertisement)
        }
        let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
        if !member.role.is_voting() || !member.health.can_vote() {
            return Err(MembershipError::InvalidState)
        }
        if term < self.term || term == self.term && self.leader.is_some_and(|leader| leader != node) {
            return Err(MembershipError::SplitBrain)
        }
        if term > self.term {
            self.term = term;
            self.voted_for = Some(node);
        }
        self.member_mut(node)?.reachable = true;
        self.leader = Some(node);
        Ok(())
    }

    pub fn propose_membership_change(
        &mut self,
        leader: NodeId,
        operation: MembershipOperation,
    ) -> Result<ConsensusProposal, MembershipError> {
        self.ensure_write_ready(Some(leader))?;
        if self.pending.is_some() {
            return Err(MembershipError::ProposalPending)
        }
        self.validate_operation(operation)?;
        let index = self.next_index;
        self.next_index = self.next_index.checked_add(1).ok_or(MembershipError::InvalidConfiguration)?;
        let mut acknowledgements = [None; MAX_MEMBERSHIP_REGISTRY];
        let slot = acknowledgements
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(MembershipError::Capacity)?;
        *slot = Some(leader);
        let acknowledged: u16 = 1;
        let pending = PendingProposal {
            index,
            term: self.term,
            epoch: self.epoch,
            operation,
            acknowledgements,
        };
        let committed = acknowledged >= self.quorum_policy.required_votes;
        self.pending = Some(pending);
        let proposal = ConsensusProposal {
            index,
            term: self.term,
            epoch: self.epoch,
            acknowledgements: acknowledged,
            required_votes: self.quorum_policy.required_votes,
            committed,
        };
        if committed {
            self.commit_pending()?;
        }
        Ok(proposal)
    }

    pub fn propose_membership_change_with_admission<const CAPACITY: usize>(
        &mut self,
        leader: NodeId,
        operation: MembershipOperation,
        admission: &mut AdmissionController<CAPACITY>,
        priority: AdmissionPriority,
    ) -> Result<ConsensusProposal, MembershipError> {
        let outcome = admission.admit(WorkClass::MembershipChange, priority);
        let Some(lease) = outcome.lease() else {
            return Err(MembershipError::Admission(outcome))
        };
        let result = self.propose_membership_change(leader, operation);
        let _ = admission.finish(lease);
        result
    }

    pub fn acknowledge_proposal(
        &mut self,
        index: u64,
        node: NodeId,
    ) -> Result<Option<ConsensusCommit>, MembershipError> {
        let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
        if !member.role.is_voting() || !member.reachable || !member.health.can_vote() {
            return Err(MembershipError::QuorumUnavailable)
        }
        let pending = self.pending.as_mut().ok_or(MembershipError::ProposalNotFound)?;
        if pending.index != index {
            return Err(MembershipError::ProposalNotFound)
        }
        if pending.acknowledgements.iter().flatten().any(|ack| *ack == node) {
            return Err(MembershipError::AlreadyAcknowledged)
        }
        let slot = pending
            .acknowledgements
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(MembershipError::Capacity)?;
        *slot = Some(node);
        if pending.acknowledgements.iter().flatten().count() as u16
            >= self.quorum_policy.required_votes
        {
            let commit = self.commit_pending()?;
            Ok(Some(commit))
        } else {
            Ok(None)
        }
    }

    pub fn pending_proposal(&self) -> Option<ConsensusProposal> {
        self.pending.map(|pending| ConsensusProposal {
            index: pending.index,
            term: pending.term,
            epoch: pending.epoch,
            acknowledgements: pending.acknowledgements.iter().flatten().count() as u16,
            required_votes: self.quorum_policy.required_votes,
            committed: false,
        })
    }

    pub fn propagate_epoch(
        &self,
        consumers: &mut [&mut dyn MembershipEpochConsumer],
    ) -> Result<(), MembershipError> {
        propagate_membership_epoch(self.epoch, consumers)
    }

    pub fn save_to_synfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<synos_synfs::TransactionCommit, MembershipError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        for directory in ["/system", "/system/cluster"] {
            match transaction.create_directory(directory, true) {
                Ok(_) | Err(synos_synfs::Error::AlreadyExists) => {}
                Err(_) => return Err(MembershipError::Persistence),
            }
        }
        transaction
            .write(MEMBERSHIP_STATE_FILE, &staging[..length])
            .map_err(|_| MembershipError::Persistence)?;
        transaction.commit().map_err(|_| MembershipError::Persistence)
    }

    pub const fn encoded_len() -> usize {
        MEMBERSHIP_HEADER_BYTES + MEMBERS * MEMBERSHIP_RECORD_BYTES + 8
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, MembershipError> {
        let required = Self::encoded_len();
        if output.len() < required {
            return Err(MembershipError::BufferTooSmall { required })
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(MEMBERSHIP_MAGIC);
        output[8..10].copy_from_slice(&MEMBERSHIP_FORMAT_VERSION.to_le_bytes());
        output[10..12].copy_from_slice(&self.quorum_policy.voting_members.to_le_bytes());
        output[12..14].copy_from_slice(&self.quorum_policy.required_votes.to_le_bytes());
        output[16..32].copy_from_slice(&self.cluster.raw());
        output[32..40].copy_from_slice(&self.epoch.to_le_bytes());
        output[40..48].copy_from_slice(&self.generation.to_le_bytes());
        output[48..56].copy_from_slice(&self.term.to_le_bytes());
        output[56..60].copy_from_slice(&self.leader.map_or(0, NodeId::raw).to_le_bytes());
        output[60..64].copy_from_slice(&self.voted_for.map_or(0, NodeId::raw).to_le_bytes());
        output[64..72].copy_from_slice(&self.commit_index.to_le_bytes());
        output[72..80].copy_from_slice(&self.next_index.to_le_bytes());
        for (index, member) in self.members.iter().enumerate() {
            if let Some(member) = member {
                encode_member(member, &mut output[MEMBERSHIP_HEADER_BYTES + index * MEMBERSHIP_RECORD_BYTES..]);
            }
        }
        let checksum_offset = required - 8;
        let checksum = membership_checksum(&output[..required]);
        output[checksum_offset..required].copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, MembershipError> {
        let required = Self::encoded_len();
        if input.len() < required
            || &input[..8] != MEMBERSHIP_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != MEMBERSHIP_FORMAT_VERSION
        {
            return Err(MembershipError::Corrupt)
        }
        let checksum_offset = required - 8;
        let stored = u64::from_le_bytes(input[checksum_offset..required].try_into().map_err(|_| MembershipError::Corrupt)?);
        if membership_checksum(&input[..required]) != stored {
            return Err(MembershipError::Corrupt)
        }
        let cluster = ClusterId::new(input[16..32].try_into().map_err(|_| MembershipError::Corrupt)?)
            .map_err(|_| MembershipError::Corrupt)?;
        let leader = decode_node(u32::from_le_bytes(input[56..60].try_into().map_err(|_| MembershipError::Corrupt)?));
        let voted_for = decode_node(u32::from_le_bytes(input[60..64].try_into().map_err(|_| MembershipError::Corrupt)?));
        let quorum_policy = QuorumPolicy {
            voting_members: u16::from_le_bytes(input[10..12].try_into().map_err(|_| MembershipError::Corrupt)?),
            required_votes: u16::from_le_bytes(input[12..14].try_into().map_err(|_| MembershipError::Corrupt)?),
        };
        let mut registry = Self::new(cluster, quorum_policy)?;
        registry.epoch = u64::from_le_bytes(input[32..40].try_into().map_err(|_| MembershipError::Corrupt)?);
        registry.generation = u64::from_le_bytes(input[40..48].try_into().map_err(|_| MembershipError::Corrupt)?);
        registry.term = u64::from_le_bytes(input[48..56].try_into().map_err(|_| MembershipError::Corrupt)?);
        registry.leader = leader;
        registry.voted_for = voted_for;
        registry.commit_index = u64::from_le_bytes(input[64..72].try_into().map_err(|_| MembershipError::Corrupt)?);
        registry.next_index = u64::from_le_bytes(input[72..80].try_into().map_err(|_| MembershipError::Corrupt)?);
        if registry.epoch == 0 || registry.generation == 0 || registry.next_index == 0 {
            return Err(MembershipError::Corrupt)
        }
        for index in 0..MEMBERS {
            let start = MEMBERSHIP_HEADER_BYTES + index * MEMBERSHIP_RECORD_BYTES;
            if input[start] == 1 {
                let member = decode_member(&input[start..start + MEMBERSHIP_RECORD_BYTES])?;
                if registry.members.iter().flatten().any(|old| old.node == member.node || old.fingerprint == member.fingerprint) {
                    return Err(MembershipError::Corrupt)
                }
                registry.members[index] = Some(member);
            } else if input[start] != 0 {
                return Err(MembershipError::Corrupt)
            }
        }
        let voting = registry.members.iter().flatten().filter(|member| member.role.is_voting()).count() as u16;
        if voting == 0
            || voting > registry.quorum_policy.voting_members
            || registry.leader.is_some_and(|leader| registry.member(leader).is_none())
            || registry.leader.is_some_and(|leader| registry.member(leader).is_some_and(|member| !member.role.is_voting()))
            || registry.voted_for.is_some_and(|node| registry.member(node).is_none())
        {
            return Err(MembershipError::Corrupt)
        }
        Ok(registry)
    }

    fn commit_pending(&mut self) -> Result<ConsensusCommit, MembershipError> {
        let pending = self.pending.take().ok_or(MembershipError::ProposalNotFound)?;
        let acknowledgements = pending.acknowledgements.iter().flatten().count() as u16;
        if acknowledgements < self.quorum_policy.required_votes {
            self.pending = Some(pending);
            return Err(MembershipError::QuorumUnavailable)
        }
        let slot = self
            .committed
            .iter()
            .position(Option::is_none)
            .ok_or(MembershipError::Capacity)?;
        if let Err(error) = self.apply_operation(pending.operation) {
            self.pending = Some(pending);
            return Err(error)
        }
        self.committed[slot] = Some(CommittedOperation {
            index: pending.index,
            term: pending.term,
            epoch: self.epoch,
            operation: pending.operation,
        });
        self.commit_index = pending.index;
        Ok(ConsensusCommit {
            index: pending.index,
            term: pending.term,
            epoch: self.epoch,
            acknowledgements,
            required_votes: self.quorum_policy.required_votes,
        })
    }

    fn apply_operation(&mut self, operation: MembershipOperation) -> Result<(), MembershipError> {
        self.validate_operation(operation)?;
        self.epoch = self.epoch.checked_add(1).ok_or(MembershipError::InvalidConfiguration)?;
        self.generation = self.generation.checked_add(1).ok_or(MembershipError::InvalidConfiguration)?;
        match operation {
            MembershipOperation::Add(spec) => {
                self.validate_add(spec)?;
                self.insert_member(spec, MemberHealth::Joining, false, 0)?;
            }
            MembershipOperation::Remove(node) => {
                let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
                if member.role.is_voting() && self.quorum_policy.voting_members <= self.quorum_policy.required_votes {
                    return Err(MembershipError::InvalidChange)
                }
                let slot = self.members.iter_mut().find(|entry| entry.is_some_and(|member| member.node == node)).ok_or(MembershipError::UnknownNode)?;
                *slot = None;
                if self.leader == Some(node) {
                    self.leader = None;
                    self.voted_for = None;
                }
            }
            MembershipOperation::ChangeRole { node, role } => {
                let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
                if member.role.is_voting() && !role.is_voting() && self.quorum_policy.voting_members <= self.quorum_policy.required_votes {
                    return Err(MembershipError::InvalidChange)
                }
                self.member_mut(node)?.role = role;
            }
            MembershipOperation::Fence(node) => {
                let member = self.member_mut(node)?;
                member.health = MemberHealth::Fenced;
                member.reachable = false;
                if self.leader == Some(node) {
                    self.leader = None;
                }
            }
        }
        for member in self.members.iter_mut().flatten() {
            member.membership_epoch = self.epoch;
            member.generation = self.generation;
        }
        Ok(())
    }

    fn validate_operation(&self, operation: MembershipOperation) -> Result<(), MembershipError> {
        match operation {
            MembershipOperation::Add(spec) => self.validate_add(spec),
            MembershipOperation::Remove(node) | MembershipOperation::Fence(node) => {
                let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
                if member.role.is_voting()
                    && self.members.iter().flatten().filter(|member| member.role.is_voting()).count() as u16
                        <= self.quorum_policy.required_votes
                {
                    return Err(MembershipError::InvalidChange)
                }
                Ok(())
            }
            MembershipOperation::ChangeRole { node, role } => {
                let member = self.member(node).ok_or(MembershipError::UnknownNode)?;
                if member.role == role {
                    return Err(MembershipError::InvalidChange)
                }
                if member.role.is_voting() && !role.is_voting()
                    && self.members.iter().flatten().filter(|member| member.role.is_voting()).count() as u16
                        <= self.quorum_policy.required_votes
                {
                    return Err(MembershipError::InvalidChange)
                }
                Ok(())
            }
        }
    }

    fn validate_add(&self, spec: MemberSpec) -> Result<(), MembershipError> {
        if spec.fingerprint == [0; 32] {
            return Err(MembershipError::InvalidConfiguration)
        }
        if self.members.iter().flatten().any(|member| member.node == spec.node || member.fingerprint == spec.fingerprint) {
            return Err(MembershipError::DuplicateIdentity)
        }
        if spec.role.is_voting()
            && self.members.iter().flatten().filter(|member| member.role.is_voting()).count() as u16
                >= self.quorum_policy.voting_members
        {
            return Err(MembershipError::Capacity)
        }
        if self.members.iter().all(Option::is_some) {
            return Err(MembershipError::Capacity)
        }
        Ok(())
    }

    fn insert_member(
        &mut self,
        spec: MemberSpec,
        health: MemberHealth,
        reachable: bool,
        now_us: u64,
    ) -> Result<RegistryMember, MembershipError> {
        let member = RegistryMember {
            node: spec.node,
            role: spec.role,
            health,
            reachable,
            fingerprint: spec.fingerprint,
            endpoint: spec.endpoint,
            capacity: spec.capacity,
            zone: spec.zone,
            rack: spec.rack,
            generation: self.generation,
            last_seen_generation: self.generation,
            membership_epoch: self.epoch,
            last_seen_us: now_us,
            sequence: 0,
        };
        let slot = self.members.iter_mut().find(|entry| entry.is_none()).ok_or(MembershipError::Capacity)?;
        *slot = Some(member);
        Ok(member)
    }

    fn member_mut(&mut self, node: NodeId) -> Result<&mut RegistryMember, MembershipError> {
        self.members.iter_mut().flatten().find(|member| member.node == node).ok_or(MembershipError::UnknownNode)
    }

    fn ensure_write_ready(&self, leader: Option<NodeId>) -> Result<(), MembershipError> {
        if !self.quorum().has_quorum {
            return Err(MembershipError::QuorumUnavailable)
        }
        let expected = leader.ok_or(MembershipError::NoLeader)?;
        if self.leader != Some(expected) {
            return Err(MembershipError::NotLeader)
        }
        let member = self.member(expected).ok_or(MembershipError::NoLeader)?;
        if !member.reachable || !member.health.can_vote() {
            return Err(MembershipError::QuorumUnavailable)
        }
        Ok(())
    }
}

pub trait MembershipEpochConsumer {
    fn apply_membership_epoch(&mut self, epoch: u64) -> Result<(), MembershipError>;
}

pub fn propagate_membership_epoch(
    epoch: u64,
    consumers: &mut [&mut dyn MembershipEpochConsumer],
) -> Result<(), MembershipError> {
    if epoch == 0 {
        return Err(MembershipError::InvalidConfiguration)
    }
    for consumer in consumers {
        consumer.apply_membership_epoch(epoch)?;
    }
    Ok(())
}

pub fn propagate_membership_epoch_with_admission<const CAPACITY: usize>(
    epoch: u64,
    consumers: &mut [&mut dyn MembershipEpochConsumer],
    admission: &mut AdmissionController<CAPACITY>,
    priority: AdmissionPriority,
) -> Result<(), MembershipError> {
    let outcome = admission.admit(WorkClass::ControlPlaneFanout, priority);
    let Some(lease) = outcome.lease() else {
        return Err(MembershipError::Admission(outcome))
    };
    let result = propagate_membership_epoch(epoch, consumers);
    let _ = admission.finish(lease);
    result
}

fn decode_node(raw: u32) -> Option<NodeId> {
    NodeId::new(raw)
}

fn encode_member(member: &RegistryMember, output: &mut [u8]) {
    output[0] = 1;
    output[1..5].copy_from_slice(&member.node.raw().to_le_bytes());
    output[5] = member.role as u8;
    output[6] = member.health as u8;
    output[7] = u8::from(member.reachable);
    output[8..16].copy_from_slice(&member.capacity.to_le_bytes());
    output[16] = member.zone.len;
    output[17..49].copy_from_slice(&member.zone.bytes);
    output[49] = member.rack.len;
    output[50..82].copy_from_slice(&member.rack.bytes);
    output[82..114].copy_from_slice(&member.fingerprint);
    output[114..122].copy_from_slice(&member.generation.to_le_bytes());
    output[122..130].copy_from_slice(&member.last_seen_generation.to_le_bytes());
    output[130..138].copy_from_slice(&member.membership_epoch.to_le_bytes());
    output[138..146].copy_from_slice(&member.last_seen_us.to_le_bytes());
    output[146..154].copy_from_slice(&member.sequence.to_le_bytes());
    let endpoint_len = member.endpoint.as_str().len() as u16;
    output[154..156].copy_from_slice(&endpoint_len.to_le_bytes());
    output[156..156 + endpoint_len as usize].copy_from_slice(member.endpoint.as_str().as_bytes());
}

fn decode_member(input: &[u8]) -> Result<RegistryMember, MembershipError> {
    let node = NodeId::new(u32::from_le_bytes(input[1..5].try_into().map_err(|_| MembershipError::Corrupt)?)).ok_or(MembershipError::Corrupt)?;
    let role = MemberRole::from_raw(input[5]).ok_or(MembershipError::Corrupt)?;
    let health = MemberHealth::from_raw(input[6]).ok_or(MembershipError::Corrupt)?;
    if input[7] > 1 || input[16] as usize > MAX_MEMBERSHIP_LABEL_BYTES || input[49] as usize > MAX_MEMBERSHIP_LABEL_BYTES {
        return Err(MembershipError::Corrupt)
    }
    let endpoint_len = u16::from_le_bytes(input[154..156].try_into().map_err(|_| MembershipError::Corrupt)?) as usize;
    if endpoint_len == 0 || endpoint_len > crate::MAX_ADMISSION_ENDPOINT_BYTES {
        return Err(MembershipError::Corrupt)
    }
    let endpoint = AdmissionEndpoint::new(core::str::from_utf8(&input[156..156 + endpoint_len]).map_err(|_| MembershipError::Corrupt)?)
        .map_err(|_| MembershipError::Corrupt)?;
    Ok(RegistryMember {
        node,
        role,
        health,
        reachable: input[7] != 0,
        fingerprint: input[82..114].try_into().map_err(|_| MembershipError::Corrupt)?,
        endpoint,
        capacity: u64::from_le_bytes(input[8..16].try_into().map_err(|_| MembershipError::Corrupt)?),
        zone: decode_label(input[16], &input[17..49])?,
        rack: decode_label(input[49], &input[50..82])?,
        generation: u64::from_le_bytes(input[114..122].try_into().map_err(|_| MembershipError::Corrupt)?),
        last_seen_generation: u64::from_le_bytes(input[122..130].try_into().map_err(|_| MembershipError::Corrupt)?),
        membership_epoch: u64::from_le_bytes(input[130..138].try_into().map_err(|_| MembershipError::Corrupt)?),
        last_seen_us: u64::from_le_bytes(input[138..146].try_into().map_err(|_| MembershipError::Corrupt)?),
        sequence: u64::from_le_bytes(input[146..154].try_into().map_err(|_| MembershipError::Corrupt)?),
    })
}

fn decode_label(len: u8, bytes: &[u8]) -> Result<MembershipLabel, MembershipError> {
    let len = len as usize;
    if len > MAX_MEMBERSHIP_LABEL_BYTES || core::str::from_utf8(&bytes[..len]).is_err() {
        return Err(MembershipError::Corrupt)
    }
    let mut output = MembershipLabel::EMPTY;
    output.len = len as u8;
    output.bytes.copy_from_slice(bytes);
    Ok(output)
}

fn membership_checksum(input: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for (index, byte) in input.iter().enumerate() {
        if index >= input.len().saturating_sub(8) {
            continue
        }
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    hash
}
