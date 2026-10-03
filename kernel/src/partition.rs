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
#[repr(C)]
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

    pub fn online(self) -> CpuMask {
        let mut words = [0; 2];
        unsafe { ghostos_core_partition_get_online(&self, words.as_mut_ptr()) };
        CpuMask::from_words(words[0], words[1])
    }

    pub fn isolated(self) -> CpuMask {
        let mut words = [0; 2];
        unsafe { ghostos_core_partition_get_isolated(&self, words.as_mut_ptr()) };
        CpuMask::from_words(words[0], words[1])
    }

    pub fn housekeeping(self) -> CpuMask {
        let mut words = [0; 2];
        unsafe { ghostos_core_partition_get_housekeeping(&self, words.as_mut_ptr()) };
        CpuMask::from_words(words[0], words[1])
    }

    pub fn is_online(self, cpu: CpuId) -> bool {
        unsafe { ghostos_core_partition_is_online(&self, cpu.raw()) }
    }

    pub fn is_isolated(self, cpu: CpuId) -> bool {
        unsafe { ghostos_core_partition_is_isolated(&self, cpu.raw()) }
    }

    pub fn accepts_kernel_work(self, cpu: CpuId) -> bool {
        unsafe { ghostos_core_partition_accepts_kernel_work(&self, cpu.raw()) }
    }

    pub fn accepts_timer(self, cpu: CpuId) -> bool {
        unsafe { ghostos_core_partition_accepts_timer(&self, cpu.raw()) }
    }

    pub fn accepts_ipc(self, cpu: CpuId) -> bool {
        unsafe { ghostos_core_partition_accepts_ipc(&self, cpu.raw()) }
    }

    pub fn set_online(&mut self, online: CpuMask) -> Result<(), CorePartitionError> {
        let words = online.raw_words();
        match unsafe { ghostos_core_partition_set_online(self, words.as_ptr()) } {
            0 => Ok(()),
            1 => Err(CorePartitionError::EmptyMask),
            _ => Err(CorePartitionError::OfflineCore),
        }
    }

    pub fn isolate(&mut self, cpus: CpuMask) -> Result<(), CorePartitionError> {
        let words = cpus.raw_words();
        match unsafe { ghostos_core_partition_isolate(self, words.as_ptr()) } {
            0 => Ok(()),
            1 => Err(CorePartitionError::EmptyMask),
            2 => Err(CorePartitionError::OfflineCore),
            _ => Err(CorePartitionError::NoHousekeepingCore),
        }
    }

    pub fn release(&mut self, cpus: CpuMask) {
        let words = cpus.raw_words();
        unsafe { ghostos_core_partition_release(self, words.as_ptr()) };
    }
}

unsafe extern "C" {
    fn ghostos_core_partition_get_online(partition: *const CorePartition, words: *mut u64);
    fn ghostos_core_partition_get_isolated(partition: *const CorePartition, words: *mut u64);
    fn ghostos_core_partition_get_housekeeping(partition: *const CorePartition, words: *mut u64);
    fn ghostos_core_partition_is_online(partition: *const CorePartition, cpu: u8) -> bool;
    fn ghostos_core_partition_is_isolated(partition: *const CorePartition, cpu: u8) -> bool;
    fn ghostos_core_partition_accepts_kernel_work(partition: *const CorePartition, cpu: u8) -> bool;
    fn ghostos_core_partition_accepts_timer(partition: *const CorePartition, cpu: u8) -> bool;
    fn ghostos_core_partition_accepts_ipc(partition: *const CorePartition, cpu: u8) -> bool;
    fn ghostos_core_partition_set_online(partition: *mut CorePartition, online: *const u64) -> u32;
    fn ghostos_core_partition_isolate(partition: *mut CorePartition, cpus: *const u64) -> u32;
    fn ghostos_core_partition_release(partition: *mut CorePartition, cpus: *const u64);
}

const _: () = {
    assert!(core::mem::size_of::<CorePartition>() == 32);
    assert!(core::mem::offset_of!(CorePartition, isolated) == 16);
};

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
