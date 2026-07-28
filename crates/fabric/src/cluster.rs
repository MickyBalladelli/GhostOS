use crate::{
    Error, NodeId,
    dsm::CoherenceDirectory,
    memory::{GlobalAddressSpace, LeaseTable},
};

pub const MAX_CLUSTER_NODES: usize = 64;
pub const MAX_HEARTBEAT_PERIOD_US: u32 = 999;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Heartbeat {
    pub node: NodeId,
    pub sequence: u32,
    pub sent_at_us: u64,
}

impl Heartbeat {
    pub const WIRE_BYTES: usize = 16;

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(&self.node.raw().to_be_bytes());
        bytes[4..8].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.sent_at_us.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; Self::WIRE_BYTES]) -> Result<Self, Error> {
        let node = NodeId::new(u32::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))
        .ok_or(Error::CorruptPacket)?;
        Ok(Self {
            node,
            sequence: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            sent_at_us: u64::from_be_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11],
                bytes[12], bytes[13], bytes[14], bytes[15],
            ]),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeState {
    Vacant,
    Alive,
    Failed,
}

#[derive(Clone, Copy)]
struct NodeHealth {
    node: Option<NodeId>,
    state: NodeState,
    last_sequence: u32,
    last_seen_us: u64,
}

impl NodeHealth {
    const EMPTY: Self = Self {
        node: None,
        state: NodeState::Vacant,
        last_sequence: 0,
        last_seen_us: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeFailure {
    pub node: NodeId,
    pub detected_at_us: u64,
    pub silence_us: u64,
}

/// Fixed-capacity, sub-millisecond heartbeat failure detector.
///
/// The caller drives `due` and `observe` from a NIC timer. Detection occurs
/// after `missed_limit` consecutive heartbeat periods.
pub struct HeartbeatMonitor<const CAPACITY: usize = MAX_CLUSTER_NODES> {
    local: NodeId,
    period_us: u32,
    missed_limit: u8,
    next_sequence: u32,
    next_send_us: u64,
    nodes: [NodeHealth; CAPACITY],
}

impl<const CAPACITY: usize> HeartbeatMonitor<CAPACITY> {
    pub fn new(
        local: NodeId,
        period_us: u32,
        missed_limit: u8,
        now_us: u64,
    ) -> Result<Self, Error> {
        if period_us == 0 || period_us > MAX_HEARTBEAT_PERIOD_US || missed_limit == 0 {
            return Err(Error::InvalidRange)
        }
        Ok(Self {
            local,
            period_us,
            missed_limit,
            next_sequence: 1,
            next_send_us: now_us,
            nodes: [NodeHealth::EMPTY; CAPACITY],
        })
    }

    pub fn add_node(&mut self, node: NodeId, now_us: u64) -> Result<(), Error> {
        if node == self.local {
            return Err(Error::InvalidDevice)
        }
        if let Some(entry) = self.nodes.iter_mut().find(|entry| entry.node == Some(node)) {
            entry.state = NodeState::Alive;
            entry.last_seen_us = now_us;
            return Ok(())
        }
        let entry = self
            .nodes
            .iter_mut()
            .find(|entry| entry.node.is_none())
            .ok_or(Error::Capacity)?;
        *entry = NodeHealth {
            node: Some(node),
            state: NodeState::Alive,
            last_sequence: 0,
            last_seen_us: now_us,
        };
        Ok(())
    }

    pub fn due(&mut self, now_us: u64) -> Option<Heartbeat> {
        if now_us < self.next_send_us {
            return None
        }
        let heartbeat = Heartbeat {
            node: self.local,
            sequence: self.next_sequence,
            sent_at_us: now_us,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_send_us = now_us.saturating_add(self.period_us as u64);
        Some(heartbeat)
    }

    pub fn observe(&mut self, heartbeat: Heartbeat, received_at_us: u64) -> Result<(), Error> {
        let entry = self
            .nodes
            .iter_mut()
            .find(|entry| entry.node == Some(heartbeat.node))
            .ok_or(Error::DeviceNotFound)?;
        if entry.last_sequence != 0
            && heartbeat.sequence.wrapping_sub(entry.last_sequence) as i32 <= 0
        {
            return Ok(())
        }
        entry.last_sequence = heartbeat.sequence;
        entry.last_seen_us = received_at_us;
        entry.state = NodeState::Alive;
        Ok(())
    }

    pub fn detect(&mut self, now_us: u64) -> Option<NodeFailure> {
        let timeout = self.period_us as u64 * self.missed_limit as u64;
        let entry = self.nodes.iter_mut().find(|entry| {
            entry.state == NodeState::Alive
                && now_us.saturating_sub(entry.last_seen_us) >= timeout
        })?;
        entry.state = NodeState::Failed;
        Some(NodeFailure {
            node: entry.node.expect("occupied health entry"),
            detected_at_us: now_us,
            silence_us: now_us.saturating_sub(entry.last_seen_us),
        })
    }

    pub fn state(&self, node: NodeId) -> Option<NodeState> {
        self.nodes
            .iter()
            .find(|entry| entry.node == Some(node))
            .map(|entry| entry.state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoverySummary {
    pub failed_node: NodeId,
    pub leases_released: usize,
    pub coherence_pages_recovered: usize,
}

/// Applies one failure decision to address routing, memory leases, and page
/// coherence. Pools with mirrors resolve to the mirror immediately afterward.
pub fn recover_failed_node<
    const POOLS: usize,
    const OVERRIDES: usize,
    const LEASES: usize,
    const PAGES: usize,
>(
    failure: NodeFailure,
    space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
    leases: &mut LeaseTable<LEASES>,
    coherence: &mut CoherenceDirectory<PAGES>,
) -> Result<RecoverySummary, Error> {
    space.mark_node_failed(failure.node)?;
    Ok(RecoverySummary {
        failed_node: failure.node,
        leases_released: leases.release_node(failure.node),
        coherence_pages_recovered: coherence.fail_node(failure.node)?,
    })
}
