use crate::task::{CpuId, CpuMask};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorePartitionError {
    EmptyMask,
    OfflineCore,
    NoHousekeepingCore,
}

/// CPU ownership policy for the microkernel scheduler.
///
/// Isolated CPUs are deliberately absent from the scheduler's runnable mask.
/// A real-time workload can pin itself to those CPUs while timer, IPC, and
/// interrupt work remains on the housekeeping CPUs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorePartition {
    online: CpuMask,
    isolated: CpuMask,
}

impl CorePartition {
    pub const fn new() -> Self {
        Self {
            online: CpuMask::CPU0,
            isolated: CpuMask::EMPTY,
        }
    }

    pub const fn online(self) -> CpuMask {
        self.online
    }

    pub const fn isolated(self) -> CpuMask {
        self.isolated
    }

    pub const fn housekeeping(self) -> CpuMask {
        self.online.difference(self.isolated)
    }

    pub const fn is_online(self, cpu: CpuId) -> bool {
        self.online.contains(cpu)
    }

    pub const fn is_isolated(self, cpu: CpuId) -> bool {
        self.isolated.contains(cpu)
    }

    pub const fn accepts_kernel_work(self, cpu: CpuId) -> bool {
        self.housekeeping().contains(cpu)
    }

    pub const fn accepts_timer(self, cpu: CpuId) -> bool {
        self.accepts_kernel_work(cpu)
    }

    pub const fn accepts_ipc(self, cpu: CpuId) -> bool {
        self.accepts_kernel_work(cpu)
    }

    pub fn set_online(&mut self, online: CpuMask) -> Result<(), CorePartitionError> {
        if online.is_empty() {
            return Err(CorePartitionError::EmptyMask)
        }
        if !self.isolated.difference(online).is_empty() {
            return Err(CorePartitionError::OfflineCore)
        }
        self.online = online;
        Ok(())
    }

    pub fn isolate(&mut self, cpus: CpuMask) -> Result<(), CorePartitionError> {
        if cpus.is_empty() {
            return Err(CorePartitionError::EmptyMask)
        }
        if !cpus.difference(self.online).is_empty() {
            return Err(CorePartitionError::OfflineCore)
        }
        let housekeeping = self.online.difference(self.isolated.union(cpus));
        if housekeeping.is_empty() {
            return Err(CorePartitionError::NoHousekeepingCore)
        }
        self.isolated = self.isolated.union(cpus);
        Ok(())
    }

    pub fn release(&mut self, cpus: CpuMask) {
        self.isolated = self.isolated.difference(cpus)
    }
}

impl Default for CorePartition {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{CorePartition, CorePartitionError};
    use crate::task::{CpuMask, CpuId};

    #[test]
    fn leaves_one_housekeeping_core() {
        let mut partition = CorePartition::new();
        partition
            .set_online(CpuMask::from_raw(0b111))
            .expect("online CPUs");
        partition
            .isolate(CpuMask::from_raw(0b110))
            .expect("isolated CPUs");
        assert_eq!(partition.housekeeping(), CpuMask::CPU0);
        assert!(partition.is_isolated(CpuId::new(1).unwrap()));
    }

    #[test]
    fn refuses_to_isolate_the_last_housekeeping_core() {
        let mut partition = CorePartition::new();
        assert_eq!(
            partition.isolate(CpuMask::CPU0),
            Err(CorePartitionError::NoHousekeepingCore)
        );
    }
}
