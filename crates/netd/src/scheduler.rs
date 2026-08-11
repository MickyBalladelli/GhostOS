use synos_time_sync::MonotonicClock;
use synos_observability::{field, BatchController, BatchDecision, EventField, EventKind, ProducerPolicy};
use synos_numa::{NumaDecision, NumaPlacement, NumaReport, NumaTopology, PlacementKind};

use crate::memory::SharedMemory;
use crate::service::{ClientChannel, NetworkDaemon, ServiceError, SocketBackend};
use crate::stack::{NetworkPoller, PollActivity};
use crate::{DhcpClient, DhcpError, DhcpLeaseRuntime, DhcpTransport};

pub const DEFAULT_SOCKET_REQUEST_BUDGET: usize = 4;
pub const DEFAULT_SOCKET_INGRESS_BUDGET: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkServiceActivity {
    pub now_ms: u64,
    pub stack: PollActivity,
    pub dhcp_retries: u64,
    pub dhcp_error: Option<DhcpError>,
    pub socket_requests: usize,
    pub service_error: Option<ServiceError>,
    pub batch: BatchDecision,
    pub interrupt_placement: NumaDecision,
}

/// Bounded coordinator for the Ring 3 network service.
///
/// Bounded stack work and one DHCP state-machine step run per scheduler turn.
/// Socket IPC work then receives a fixed budget, even when DHCP is retrying or
/// reports a failure.
pub struct NetworkServiceScheduler {
    socket_request_budget: usize,
    stack_ingress_budget: usize,
    controller: BatchController,
    numa: NumaPlacement,
    interrupt_cpu: Option<u16>,
    interrupt_node: Option<u8>,
}

impl NetworkServiceScheduler {
    pub const fn new(socket_request_budget: usize) -> Self {
        Self {
            socket_request_budget,
            stack_ingress_budget: DEFAULT_SOCKET_INGRESS_BUDGET,
            controller: BatchController::new(ProducerPolicy::NETWORK),
            numa: NumaPlacement::uma(),
            interrupt_cpu: None,
            interrupt_node: None,
        }
    }

    pub const fn with_budgets(
        socket_request_budget: usize,
        stack_ingress_budget: usize,
    ) -> Self {
        Self {
            socket_request_budget,
            stack_ingress_budget,
            controller: BatchController::new(ProducerPolicy::NETWORK),
            numa: NumaPlacement::uma(),
            interrupt_cpu: None,
            interrupt_node: None,
        }
    }

    pub const fn default_budget() -> usize {
        DEFAULT_SOCKET_REQUEST_BUDGET
    }

    pub const fn socket_request_budget(&self) -> usize {
        self.socket_request_budget
    }

    pub const fn stack_ingress_budget(&self) -> usize {
        self.stack_ingress_budget
    }

    pub fn configure_numa(
        &mut self,
        topology: NumaTopology,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        self.numa.set_topology(topology);
        self.interrupt_cpu = preferred_cpu;
        self.interrupt_node = preferred_node;
        self.numa
            .place(PlacementKind::NetworkInterrupt, preferred_cpu, preferred_node)
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.numa.report()
    }

    pub fn poll<C, T, R, B, M, const SOCKET_CAPACITY: usize, const RING_CAPACITY: usize>(
        &mut self,
        clock: &C,
        dhcp: &mut DhcpClient,
        transport: &mut T,
        runtime: &mut R,
        daemon: &mut NetworkDaemon<B, SOCKET_CAPACITY>,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
    ) -> NetworkServiceActivity
    where
        C: MonotonicClock,
        T: DhcpTransport,
        R: DhcpLeaseRuntime,
        B: SocketBackend + NetworkPoller,
        M: SharedMemory,
    {
        let now_ms = clock.now_us() / 1_000;
        let batch = self.controller.plan(
            self.stack_ingress_budget,
            1,
            0,
            self.socket_request_budget != 0,
        );
        let interrupt_placement = self.numa.place(
            PlacementKind::NetworkInterrupt,
            self.interrupt_cpu,
            self.interrupt_node,
        );
        synos_observability::info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::NetworkInterrupt as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, interrupt_placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, interrupt_placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, interrupt_placement.locality as u64),
        );
        let stack = daemon.backend_mut().poll_network(
            core::cmp::min(now_ms, i64::MAX as u64) as i64,
            batch.count,
        );
        let dhcp_error = dhcp.poll(now_ms, transport, runtime).err();
        let dhcp_retries = dhcp.retry_count();
        daemon.backend_mut().set_dhcp_retries(dhcp_retries);
        let mut stack = stack;
        stack.stats.dhcp_retries = dhcp_retries;
        let (socket_requests, service_error) = match daemon.process_budget_at(
            channel,
            memory,
            self.socket_request_budget,
            now_ms,
        ) {
            Ok(processed) => (processed, None),
            Err(error) => (0, Some(error)),
        };
        NetworkServiceActivity {
            now_ms,
            stack,
            dhcp_retries,
            dhcp_error,
            socket_requests,
            service_error,
            batch,
            interrupt_placement,
        }
    }

    /// Poll using the smoltcp backend as the DHCP runtime too. Successful
    /// lease and static restoration callbacks therefore update the same
    /// interface that handles socket traffic.
    pub fn poll_with_backend_runtime<
        C,
        T,
        B,
        M,
        const SOCKET_CAPACITY: usize,
        const RING_CAPACITY: usize,
    >(
        &mut self,
        clock: &C,
        dhcp: &mut DhcpClient,
        transport: &mut T,
        daemon: &mut NetworkDaemon<B, SOCKET_CAPACITY>,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
    ) -> NetworkServiceActivity
    where
        C: MonotonicClock,
        T: DhcpTransport,
        B: SocketBackend + NetworkPoller + DhcpLeaseRuntime,
        M: SharedMemory,
    {
        let now_ms = clock.now_us() / 1_000;
        let batch = self.controller.plan(
            self.stack_ingress_budget,
            1,
            0,
            self.socket_request_budget != 0,
        );
        let interrupt_placement = self.numa.place(
            PlacementKind::NetworkInterrupt,
            self.interrupt_cpu,
            self.interrupt_node,
        );
        synos_observability::info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::NetworkInterrupt as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, interrupt_placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, interrupt_placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, interrupt_placement.locality as u64),
        );
        let stack = {
            let backend = daemon.backend_mut();
            backend.poll_network(
                core::cmp::min(now_ms, i64::MAX as u64) as i64,
                batch.count,
            )
        };
        let dhcp_error = {
            let backend = daemon.backend_mut();
            dhcp.poll(now_ms, transport, backend).err()
        };
        let dhcp_retries = dhcp.retry_count();
        daemon.backend_mut().set_dhcp_retries(dhcp_retries);
        let mut stack = stack;
        stack.stats.dhcp_retries = dhcp_retries;
        let (socket_requests, service_error) = match daemon.process_budget_at(
            channel,
            memory,
            self.socket_request_budget,
            now_ms,
        ) {
            Ok(processed) => (processed, None),
            Err(error) => (0, Some(error)),
        };
        NetworkServiceActivity {
            now_ms,
            stack,
            dhcp_retries,
            dhcp_error,
            socket_requests,
            service_error,
            batch,
            interrupt_placement,
        }
    }
}

impl Default for NetworkServiceScheduler {
    fn default() -> Self {
        Self::new(DEFAULT_SOCKET_REQUEST_BUDGET)
    }
}
