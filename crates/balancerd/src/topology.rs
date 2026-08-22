use ghostos_auth::{
    accept_offer, token::CapabilityKey, ClusterId, FederatedLease, FederatedResourceOffer,
    FederationError, PeerDirectory, ResourceLender, RevocationAction, RevocationSignal,
};
use ghostos_fabric::NodeId as ClusterNodeId;
use ghostos_kernel::FederationFenceTable;

use crate::NodeOffer;

pub const DEFAULT_ARBITRATION_NODE_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadBalancingTopology {
    IntraCluster,
    InterCluster,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustScope {
    ClusterTrusted,
    ZeroTrustFederation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrimaryMechanism {
    GlobalAddressSpace,
    CapabilityGatedLease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityMechanism {
    SharedDlmAndCapabilityHandles,
    CryptographicCapabilitiesAndEncryptedMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreemptionModel {
    DynamicRebalance,
    HardRevocationAndEpochFencing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArbitrationPolicy {
    pub topology: LoadBalancingTopology,
    pub trust: TrustScope,
    pub primary: PrimaryMechanism,
    pub security: SecurityMechanism,
    pub preemption: PreemptionModel,
}

/// The executable form of the topology table in TODO.md.
pub struct ArbitrationMatrix;

impl ArbitrationMatrix {
    pub const fn policy(topology: LoadBalancingTopology) -> ArbitrationPolicy {
        match topology {
            LoadBalancingTopology::IntraCluster => ArbitrationPolicy {
                topology,
                trust: TrustScope::ClusterTrusted,
                primary: PrimaryMechanism::GlobalAddressSpace,
                security: SecurityMechanism::SharedDlmAndCapabilityHandles,
                preemption: PreemptionModel::DynamicRebalance,
            },
            LoadBalancingTopology::InterCluster => ArbitrationPolicy {
                topology,
                trust: TrustScope::ZeroTrustFederation,
                primary: PrimaryMechanism::CapabilityGatedLease,
                security: SecurityMechanism::CryptographicCapabilitiesAndEncryptedMemory,
                preemption: PreemptionModel::HardRevocationAndEpochFencing,
            },
        }
    }

    pub const fn intra_cluster() -> ArbitrationPolicy {
        Self::policy(LoadBalancingTopology::IntraCluster)
    }

    pub const fn inter_cluster() -> ArbitrationPolicy {
        Self::policy(LoadBalancingTopology::InterCluster)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryProtection {
    SharedClusterMemory,
    HardwareEncrypted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraClusterGrant {
    pub node: ClusterNodeId,
    pub node_epoch: u64,
    pub projected_load_millionths: u64,
    pub policy: ArbitrationPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterClusterGrant {
    pub provider: ClusterId,
    pub resource: u64,
    pub lease: FederatedLease,
    pub policy: ArbitrationPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArbitrationError {
    Capacity,
    Federation(FederationError),
    InsecureMemory,
    InvalidRequest,
    NoEligibleNode,
    StaleEpoch,
    Fence(ghostos_kernel::LockError),
}

#[derive(Clone, Copy)]
struct NodeLoad {
    offer: NodeOffer,
    active_threads: usize,
    failed: bool,
}

/// Fixed-capacity arbitration state shared by local placement and federation
/// admission. It does not own actors or leases; those remain in Balancer and
/// the federation lender respectively.
pub struct TopologyArbiter<const NODES: usize = DEFAULT_ARBITRATION_NODE_CAPACITY> {
    nodes: [Option<NodeLoad>; NODES],
}

impl<const NODES: usize> TopologyArbiter<NODES> {
    pub const fn new() -> Self {
        Self {
            nodes: [None; NODES],
        }
    }

    pub fn admit_intra_node(&mut self, offer: NodeOffer) -> Result<(), ArbitrationError> {
        if offer.node_epoch == 0 || offer.cpu_capacity == 0 {
            return Err(ArbitrationError::InvalidRequest);
        }
        if let Some(node) = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|node| node.offer.node == offer.node)
        {
            if offer.node_epoch < node.offer.node_epoch {
                return Err(ArbitrationError::StaleEpoch);
            }
            if offer.cpu_capacity < node.active_threads {
                return Err(ArbitrationError::Capacity);
            }
            node.offer = offer;
            node.failed = false;
            return Ok(());
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|node| node.is_none())
            .ok_or(ArbitrationError::Capacity)?;
        *slot = Some(NodeLoad {
            offer,
            active_threads: 0,
            failed: false,
        });
        Ok(())
    }

    pub fn remove_intra_node(&mut self, node: ClusterNodeId) -> Result<(), ArbitrationError> {
        let slot = self
            .nodes
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.offer.node == node))
            .ok_or(ArbitrationError::NoEligibleNode)?;
        if self.nodes[slot].is_some_and(|entry| entry.active_threads != 0) {
            return Err(ArbitrationError::Capacity);
        }
        self.nodes[slot] = None;
        Ok(())
    }

    pub fn mark_intra_node_failed(
        &mut self,
        node: ClusterNodeId,
        failed: bool,
    ) -> Result<(), ArbitrationError> {
        let state = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|state| state.offer.node == node)
            .ok_or(ArbitrationError::NoEligibleNode)?;
        state.failed = failed;
        Ok(())
    }

    /// Reserve one local compute slot using least projected load, then cache
    /// latency and node id as deterministic tie breakers.
    pub fn reserve_intra_thread(&mut self) -> Result<IntraClusterGrant, ArbitrationError> {
        let slot = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                let state = (*state)?;
                if state.failed || state.active_threads >= state.offer.cpu_capacity {
                    return None;
                }
                let projected = state.active_threads + 1;
                let load = (projected as u64)
                    .saturating_mul(1_000_000)
                    .checked_div(state.offer.cpu_capacity as u64)
                    .unwrap_or(u64::MAX);
                Some((
                    load,
                    state.offer.cache_latency_ns,
                    state.offer.node.raw(),
                    index,
                ))
            })
            .min_by_key(|entry| (entry.0, entry.1, entry.2))
            .map(|entry| entry.3)
            .ok_or(ArbitrationError::NoEligibleNode)?;
        let state = self.nodes[slot].as_mut().expect("selected node");
        state.active_threads += 1;
        let projected = state.active_threads as u64;
        let load = projected
            .saturating_mul(1_000_000)
            .checked_div(state.offer.cpu_capacity as u64)
            .unwrap_or(u64::MAX);
        Ok(IntraClusterGrant {
            node: state.offer.node,
            node_epoch: state.offer.node_epoch,
            projected_load_millionths: load,
            policy: ArbitrationMatrix::intra_cluster(),
        })
    }

    pub fn release_intra_thread(&mut self, node: ClusterNodeId) -> Result<(), ArbitrationError> {
        let state = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|state| state.offer.node == node)
            .ok_or(ArbitrationError::NoEligibleNode)?;
        state.active_threads = state.active_threads.saturating_sub(1);
        Ok(())
    }

    pub const fn inter_cluster_policy(&self) -> ArbitrationPolicy {
        ArbitrationMatrix::inter_cluster()
    }

    /// Admit a remote resource only after peer authentication, offer
    /// validation, capability issuance, and hardware-encryption confirmation.
    pub fn arbitrate_inter_cluster<const PEERS: usize, const LOANS: usize>(
        &self,
        directory: &PeerDirectory<PEERS>,
        peering_key: CapabilityKey,
        lender: &mut ResourceLender<LOANS>,
        offer: FederatedResourceOffer,
        borrower: ClusterNodeId,
        memory_protection: MemoryProtection,
        now_us: u64,
        duration_us: u64,
    ) -> Result<InterClusterGrant, ArbitrationError> {
        if memory_protection != MemoryProtection::HardwareEncrypted {
            return Err(ArbitrationError::InsecureMemory);
        }
        let lease = accept_offer(
            directory,
            peering_key,
            lender,
            offer,
            borrower,
            now_us,
            duration_us,
        )
        .map_err(ArbitrationError::Federation)?;
        Ok(InterClusterGrant {
            provider: lease.provider,
            resource: lease.capability.resource,
            lease,
            policy: ArbitrationMatrix::inter_cluster(),
        })
    }

    /// Apply a signed remote revocation and advance the matching kernel fence.
    /// The fence must already be established at prior_epoch by discovery.
    pub fn preempt_inter_cluster<const LOANS: usize, const FENCES: usize>(
        &self,
        signal: &RevocationSignal,
        key: CapabilityKey,
        active_lease: &FederatedLease,
        now_us: u64,
        lender: &mut ResourceLender<LOANS>,
        fences: &mut FederationFenceTable<FENCES>,
    ) -> Result<RevocationAction, ArbitrationError> {
        fences
            .validate(signal.provider.dlm_id(), signal.prior_epoch)
            .map_err(ArbitrationError::Fence)?;
        let action = signal
            .apply(key, active_lease, now_us, lender)
            .map_err(ArbitrationError::Federation)?;
        fences
            .advance(
                signal.provider.dlm_id(),
                signal.prior_epoch,
                signal.next_epoch,
            )
            .map_err(ArbitrationError::Fence)?;
        Ok(action)
    }
}

impl<const NODES: usize> Default for TopologyArbiter<NODES> {
    fn default() -> Self {
        Self::new()
    }
}

impl From<FederationError> for ArbitrationError {
    fn from(error: FederationError) -> Self {
        Self::Federation(error)
    }
}
