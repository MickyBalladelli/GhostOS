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
        7 => Err(ScaleError::NoTarget), 8 => Err(ScaleError::Conflict),
        9 => Err(ScaleError::InvalidId), _ => unreachable!("native scaling result"),
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

fn session_state(state: crate::SessionState) -> u8 {
    match state { crate::SessionState::Active => 0, crate::SessionState::HandoffPrepared => 1,
        crate::SessionState::HandoffAccepted => 2, crate::SessionState::Closed => 3 }
}
pub(crate) fn close(state: crate::SessionState, in_flight: u16) -> Result<(), ScaleError> {
    result(unsafe { ghostos_scale_session_close(session_state(state), in_flight) })
}
pub(crate) fn prepare(state: crate::SessionState, in_flight: u16, generation: u64,
    requested_generation: u64, sequence: u64, requested_sequence: u64) -> Result<(), ScaleError> {
    result(unsafe { ghostos_scale_prepare(session_state(state), in_flight, generation,
        requested_generation, sequence, requested_sequence) })
}
#[repr(C)]
struct Handoff { source_generation: u64, target_generation: u64, sequence: u64,
    digest: u64, source: u32, target: u32 }
pub(crate) fn same_handoff<const N: usize>(stored: &crate::HandoffRecord<N>, token: crate::HandoffToken) -> bool {
    let stored = Handoff { source_generation: stored.source_generation, target_generation: stored.target_generation,
        sequence: stored.sequence, digest: stored.digest, source: stored.source.raw(), target: stored.target.raw() };
    let token = Handoff { source_generation: token.source_generation, target_generation: token.target_generation,
        sequence: token.sequence, digest: token.snapshot_digest, source: token.source.raw(), target: token.target.raw() };
    unsafe { ghostos_scale_same_handoff(&stored, &token) }
}
const _: () = {
    assert!(core::mem::size_of::<Handoff>() == 40);
    assert!(core::mem::offset_of!(Handoff, source) == 32);
};
unsafe extern "C" {
    fn ghostos_scale_session_close(state: u8, in_flight: u16) -> i32;
    fn ghostos_scale_prepare(state: u8, in_flight: u16, generation: u64,
        requested_generation: u64, sequence: u64, requested_sequence: u64) -> i32;
    fn ghostos_scale_same_handoff(stored: *const Handoff, token: *const Handoff) -> bool;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Request { id: u64, session: u64, effect: u64, state: u8, occupied: bool }
#[repr(C)]
#[derive(Clone, Copy)]
struct Effect { effect: u64, occupied: bool }
pub(crate) fn route<const R: usize, const E: usize>(requests: &[Option<crate::RequestRecord>; R],
    effects: &[Option<crate::EffectRecord>; E], request: crate::RequestId,
    session: crate::SessionId, effect: u64) -> Result<(u8, usize), ScaleError> {
    let requests = requests.map(|record| match record {
        Some(record) => Request { id: record.id.raw(), session: record.session.raw(), effect: record.effect,
            state: match record.state { crate::RequestState::InFlight => 0, crate::RequestState::Failed => 1,
                crate::RequestState::Completed => 2 }, occupied: true },
        None => Request { id: 0, session: 0, effect: 0, state: 0, occupied: false },
    });
    let effects = effects.map(|record| match record {
        Some(record) => Effect { effect: record.effect, occupied: true },
        None => Effect { effect: 0, occupied: false },
    });
    let mut decision = 0;
    let mut index = 0;
    result(unsafe { ghostos_scale_route(requests.as_ptr(), R, effects.as_ptr(), E,
        request.raw(), session.raw(), effect, &mut decision, &mut index) })?;
    Ok((decision, index))
}
const _: () = {
    assert!(core::mem::size_of::<Request>() == 32);
    assert!(core::mem::offset_of!(Request, occupied) == 25);
    assert!(core::mem::size_of::<Effect>() == 16);
};
unsafe extern "C" {
    fn ghostos_scale_route(requests: *const Request, request_count: usize,
        effects: *const Effect, effect_count: usize, request: u64, session: u64,
        effect: u64, decision: *mut u8, index: *mut usize) -> i32;
}
