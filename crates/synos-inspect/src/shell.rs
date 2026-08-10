use syn_shell::{
    Text,
    diagnostics::{
        ClusterSnapshot, CpuSnapshot, DiagnosticSource, DiskSnapshot, HealthSnapshot, MemorySnapshot,
        MonitorSnapshot, ObsoleteSnapshot, ProcessSnapshot,
        ProcessState as ShellProcessState, UptimeSnapshot, UsersSnapshot,
    },
};
use synos_audit::{ObsolescenceReason, PackageKind};
use synos_status::Status;

use crate::{
    DeviceHealth, InspectCapability, InspectionProvider, InspectionService, ProcessId,
    ProcessState, StorageKind, View,
};

pub struct ShellInspectionSource<Provider> {
    service: InspectionService<Provider>,
    capability: InspectCapability,
    now_us: u64,
    boot_at_us: u64,
}

impl<Provider> ShellInspectionSource<Provider> {
    pub const fn new(
        service: InspectionService<Provider>,
        capability: InspectCapability,
        now_us: u64,
    ) -> Self {
        Self {
            service,
            capability,
            now_us,
            boot_at_us: 0,
        }
    }

    pub const fn service(&self) -> &InspectionService<Provider> {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut InspectionService<Provider> {
        &mut self.service
    }

    pub fn set_capability(&mut self, capability: InspectCapability) {
        self.capability = capability
    }

    pub fn set_now_us(&mut self, now_us: u64) {
        self.now_us = now_us
    }

    pub fn set_boot_at_us(&mut self, boot_at_us: u64) {
        self.boot_at_us = boot_at_us
    }

    const fn view(cluster: bool) -> View {
        if cluster { View::Cluster } else { View::Local }
    }
}

impl<Provider: InspectionProvider> DiagnosticSource for ShellInspectionSource<Provider> {
    fn memory(&mut self, cluster: bool) -> Result<MemorySnapshot, Status> {
        let report = self
            .service
            .memory(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        let total_bytes = report
            .nodes()
            .fold(0u64, |total, node| total.saturating_add(node.total_bytes));
        let free_bytes = report
            .nodes()
            .fold(0u64, |total, node| total.saturating_add(node.free_bytes));
        let cxl_bytes = report.cxl_leases().fold(0u64, |total, lease| {
            total.saturating_add(lease.range.length)
        });
        let layer2_bytes = report.dsm_allocations().fold(0u64, |total, allocation| {
            total.saturating_add(allocation.range.length)
        });
        Ok(MemorySnapshot {
            total_bytes,
            free_bytes,
            local_bytes: total_bytes
                .saturating_sub(cxl_bytes)
                .saturating_sub(layer2_bytes),
            cxl_bytes,
            layer2_bytes,
            vram_bytes: 0,
            active_leases: report.cxl_leases().count() as u64,
            failed_nodes: 0,
        })
    }

    fn disk(&mut self, cluster: bool) -> Result<DiskSnapshot, Status> {
        let report = self
            .service
            .storage(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        let mut snapshot = DiskSnapshot {
            capacity_bytes: 0,
            allocated_bytes: 0,
            synfs_used_bytes: 0,
            cow_overhead_bytes: 0,
            retained_versions: 0,
            checkpoints: 0,
            nvme_devices: 0,
            cxl_devices: 0,
            degraded_devices: 0,
            failed_devices: 0,
        };
        for device in report.devices() {
            snapshot.capacity_bytes = snapshot
                .capacity_bytes
                .saturating_add(device.capacity_bytes);
            snapshot.allocated_bytes = snapshot
                .allocated_bytes
                .saturating_add(device.allocated_bytes);
            match device.kind {
                StorageKind::Ahci => {}
                StorageKind::Nvme => {
                    snapshot.nvme_devices = snapshot.nvme_devices.saturating_add(1)
                }
                StorageKind::CxlPersistentMemory => {
                    snapshot.cxl_devices = snapshot.cxl_devices.saturating_add(1)
                }
                StorageKind::NetworkBlock => {}
            }
            match device.health {
                DeviceHealth::Online => {}
                DeviceHealth::Degraded => {
                    snapshot.degraded_devices = snapshot.degraded_devices.saturating_add(1)
                }
                DeviceHealth::Failed => {
                    snapshot.failed_devices = snapshot.failed_devices.saturating_add(1)
                }
            }
        }
        for volume in report.volumes() {
            snapshot.synfs_used_bytes = snapshot.synfs_used_bytes.saturating_add(volume.used_bytes);
            snapshot.cow_overhead_bytes = snapshot
                .cow_overhead_bytes
                .saturating_add(volume.cow_overhead_bytes);
            snapshot.retained_versions = snapshot
                .retained_versions
                .saturating_add(volume.retained_versions);
            snapshot.checkpoints = snapshot
                .checkpoints
                .saturating_add(volume.checkpoints as u64)
        }
        Ok(snapshot)
    }

    fn cpu(&mut self, cluster: bool) -> Result<CpuSnapshot, Status> {
        let report = self
            .service
            .cpu(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        let mut snapshot = CpuSnapshot {
            sample_period_us: 0,
            capacity_us: 0,
            microkernel_us: 0,
            user_daemon_us: 0,
            dsm_fault_us: 0,
            idle_us: 0,
            context_switches: 0,
            dsm_faults: 0,
        };
        for node in report.nodes() {
            snapshot.sample_period_us = snapshot.sample_period_us.max(node.sample_period_us);
            snapshot.capacity_us = snapshot.capacity_us.saturating_add(node.capacity_us());
            snapshot.microkernel_us = snapshot.microkernel_us.saturating_add(node.microkernel_us);
            snapshot.user_daemon_us = snapshot.user_daemon_us.saturating_add(node.user_daemon_us);
            snapshot.dsm_fault_us = snapshot.dsm_fault_us.saturating_add(node.dsm_fault_us);
            snapshot.idle_us = snapshot.idle_us.saturating_add(node.idle_us);
            snapshot.context_switches = snapshot
                .context_switches
                .saturating_add(node.context_switches);
            snapshot.dsm_faults = snapshot.dsm_faults.saturating_add(node.dsm_faults)
        }
        Ok(snapshot)
    }

    fn users(&mut self, cluster: bool) -> Result<UsersSnapshot, Status> {
        let report = self
            .service
            .activity(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        Ok(UsersSnapshot {
            visible_users: report.users().count() as u64,
            sessions: report.sessions().count() as u64,
            local_sessions: report.sessions().filter(|session| !session.remote).count() as u64,
            remote_sessions: report.sessions().filter(|session| session.remote).count() as u64,
            processes: report.processes().count() as u64,
        })
    }

    fn obsolete(&mut self, cluster: bool) -> Result<ObsoleteSnapshot, Status> {
        let report = self
            .service
            .obsolete(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        let mut snapshot = ObsoleteSnapshot {
            sampled_at_us: report.sampled_at_us(),
            packages: 0,
            binaries: 0,
            drivers: 0,
            deprecated: 0,
            unmaintained: 0,
            out_of_date: 0,
            nodes: 0,
        };
        for package in report.packages() {
            snapshot.packages = snapshot.packages.saturating_add(1);
            match package.kind {
                PackageKind::Binary => snapshot.binaries = snapshot.binaries.saturating_add(1),
                PackageKind::Driver => snapshot.drivers = snapshot.drivers.saturating_add(1),
            }
            match package.reason {
                ObsolescenceReason::Deprecated => {
                    snapshot.deprecated = snapshot.deprecated.saturating_add(1)
                }
                ObsolescenceReason::Unmaintained => {
                    snapshot.unmaintained = snapshot.unmaintained.saturating_add(1)
                }
                ObsolescenceReason::OutOfDate => {
                    snapshot.out_of_date = snapshot.out_of_date.saturating_add(1)
                }
            }
        }
        snapshot.nodes = report.packages().map(|package| package.node).collect::<NodeSet>().len();
        Ok(snapshot)
    }

    fn health(&mut self, cluster: bool) -> Result<HealthSnapshot, Status> {
        let report = self
            .service
            .health(self.capability, Self::view(cluster), self.now_us)
            .map_err(|error| error.status())?;
        Ok(HealthSnapshot {
            sampled_at_us: report.sampled_at_us(),
            healthy_transports: report.healthy_count() as u64,
            degraded_transports: report.degraded_count() as u64,
            failed_transports: report.failed_count() as u64,
            queue_depth: report.queue_depth(),
            queue_capacity: report.queue_capacity(),
            dropped_packets: report.dropped_packets(),
            retries: report.retries(),
            degraded_mode: report.degraded_mode(),
            status: report.status(),
        })
    }

    fn cluster(&mut self) -> Result<ClusterSnapshot, Status> {
        let report = self
            .service
            .memory(self.capability, View::Cluster, self.now_us)
            .map_err(|error| error.status())?;
        Ok(ClusterSnapshot {
            nodes: report.nodes().count() as u64,
            healthy_nodes: report.nodes().count() as u64,
            degraded_nodes: 0,
            heartbeat_period_us: 0,
            remote_pages: report.dsm_allocations().fold(0u64, |pages, allocation| {
                pages.saturating_add(allocation.resident_pages as u64)
            }),
            migrations: 0,
        })
    }

    fn process(&mut self, pid: Option<u64>) -> Result<ProcessSnapshot, Status> {
        let process = pid
            .map(|raw| ProcessId::new(raw).ok_or(Status::INVALID_ARGUMENT))
            .transpose()?;
        let sample = self
            .service
            .process(self.capability, process, self.now_us)
            .map_err(|error| error.status())?;
        Ok(ProcessSnapshot {
            pid: sample.id.raw(),
            name: Text::new(sample.name.as_str()).map_err(|_| Status::INVALID_ARGUMENT)?,
            state: match sample.state {
                ProcessState::Ready => ShellProcessState::Ready,
                ProcessState::Running => ShellProcessState::Running,
                ProcessState::Blocked => ShellProcessState::Blocked,
                ProcessState::Stopped => ShellProcessState::Stopped,
            },
            threads: sample.threads as u64,
            resident_bytes: sample.resident_bytes,
            cpu_time_us: sample.cpu_time_us,
        })
    }

    fn monitor(&mut self, interval_us: u64, samples: u64) -> Result<MonitorSnapshot, Status> {
        let cpu = self.cpu(false)?;
        let memory = self.memory(false)?;
        let busy = cpu.capacity_us.saturating_sub(cpu.idle_us);
        Ok(MonitorSnapshot {
            sample: samples,
            interval_us,
            runnable_threads: 0,
            cpu_busy_percent: if cpu.capacity_us == 0 {
                0
            } else {
                busy.saturating_mul(100) / cpu.capacity_us
            },
            memory_used_bytes: memory.total_bytes.saturating_sub(memory.free_bytes),
            remote_faults: cpu.dsm_faults,
            network_bytes: 0,
        })
    }

    fn uptime(&mut self) -> Result<UptimeSnapshot, Status> {
        Ok(UptimeSnapshot {
            uptime_us: self.now_us.saturating_sub(self.boot_at_us),
        })
    }
}

struct NodeSet {
    nodes: [Option<synos_fabric::NodeId>; 64],
    length: usize,
}

impl NodeSet {
    fn len(self) -> u64 {
        self.length as u64
    }
}

impl FromIterator<synos_fabric::NodeId> for NodeSet {
    fn from_iter<T: IntoIterator<Item = synos_fabric::NodeId>>(iter: T) -> Self {
        let mut set = Self {
            nodes: [None; 64],
            length: 0,
        };
        for node in iter {
            if set.nodes[..set.length].contains(&Some(node)) {
                continue
            }
            if set.length == set.nodes.len() {
                break
            }
            set.nodes[set.length] = Some(node);
            set.length += 1;
        }
        set
    }
}
