use crate::{InstanceId, InstanceRecord, InstanceState, ScaleError};
#[repr(C)]
#[derive(Clone, Copy)]
struct Instance {
    generation: u64, id: u32, sessions: u16, in_flight: u16, state: u8, occupied: bool,
}
impl Instance {
    fn from_record(record: Option<InstanceRecord>) -> Self {
        match record {
            Some(record) => Self { generation: record.generation, id: record.id.raw(),
                sessions: record.sessions, in_flight: record.in_flight,
                state: match record.state { InstanceState::Joining => 0, InstanceState::Ready => 1,
                    InstanceState::Draining => 2, InstanceState::Restarting => 3 }, occupied: true },
            None => Self { generation: 0, id: 0, sessions: 0, in_flight: 0, state: 0, occupied: false },
        }
    }
}
fn result(code: i32) -> Result<(), ScaleError> {
    match code {
        0 => Ok(()), 1 => Err(ScaleError::Capacity), 2 => Err(ScaleError::Duplicate),
        3 => Err(ScaleError::NotFound), 4 => Err(ScaleError::InvalidState),
        5 => Err(ScaleError::StaleGeneration), 6 => Err(ScaleError::InFlight),
        7 => Err(ScaleError::NoTarget), _ => unreachable!("native scaling result"),
    }
}
pub(crate) fn membership<const N: usize>(records: &[Option<InstanceRecord>; N],
    id: InstanceId, generation: u64, operation: u8) -> Result<usize, ScaleError> {
    let instances = records.map(Instance::from_record);
    let mut index = 0;
    result(unsafe { ghostos_scale_membership(instances.as_ptr(), N, id.raw(), generation, operation, &mut index) })?;
    Ok(index)
}
pub(crate) fn target<const N: usize>(records: &[Option<InstanceRecord>; N], owner: InstanceId) -> Result<InstanceId, ScaleError> {
    let instances = records.map(Instance::from_record);
    let mut id = 0;
    result(unsafe { ghostos_scale_target(instances.as_ptr(), N, owner.raw(), &mut id) })?;
    InstanceId::new(id).ok_or(ScaleError::InvalidId)
}
pub(crate) fn digest(bytes: &[u8]) -> u64 {
    unsafe { ghostos_scale_digest(bytes.as_ptr(), bytes.len()) }
}
const _: () = {
    assert!(core::mem::size_of::<Instance>() == 24);
    assert!(core::mem::offset_of!(Instance, occupied) == 17);
};
unsafe extern "C" {
    fn ghostos_scale_membership(instances: *const Instance, count: usize,
        id: u32, generation: u64, operation: u8, index: *mut usize) -> i32;
    fn ghostos_scale_target(instances: *const Instance, count: usize, owner: u32, target: *mut u32) -> i32;
    fn ghostos_scale_digest(bytes: *const u8, length: usize) -> u64;
}
