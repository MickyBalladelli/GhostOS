//! Synchronous, allocation-free C adapters. C never retains supplied pointers.
use super::*;
use core::mem::MaybeUninit;

#[repr(C)]
pub(super) struct Pending {
    sequence: u16,
    master: u32,
    t1: u64,
    t2: u64,
    t3: u64,
    correction: i64,
    has_t1: bool,
    has_t2: bool,
    has_t3: bool,
}

#[repr(C)]
pub(super) struct Message {
    kind: u8,
    sequence: u16,
    source: u32,
    target: u32,
    first: PtpTimestamp,
    second: PtpTimestamp,
    correction: i64,
}

impl From<PtpMessage> for Message {
    fn from(value: PtpMessage) -> Self {
        let (first, second) = match value {
            PtpMessage::Sync { origin_timestamp, .. } | PtpMessage::FollowUp { origin_timestamp, .. }
            | PtpMessage::DelayRequest { origin_timestamp, .. } => (origin_timestamp, PtpTimestamp::ZERO),
            PtpMessage::DelayResponse { request_timestamp, receive_timestamp, .. } => (request_timestamp, receive_timestamp),
        };
        Self { kind: value.message_type() as u8, sequence: value.sequence_id(), source: value.source(),
            target: value.target(), first, second, correction: value.correction_ns() }
    }
}

impl Message {
    pub fn into_message(self) -> PtpMessage {
        let Self { kind, sequence: sequence_id, source, target, first: origin_timestamp, second, correction: correction_ns } = self;
        match kind {
            0 => PtpMessage::Sync { sequence_id, source, target, origin_timestamp, correction_ns },
            8 => PtpMessage::FollowUp { sequence_id, source, target, origin_timestamp, correction_ns },
            1 => PtpMessage::DelayRequest { sequence_id, source, target, origin_timestamp, correction_ns },
            9 => PtpMessage::DelayResponse { sequence_id, source, target, request_timestamp: origin_timestamp,
                receive_timestamp: second, correction_ns },
            _ => unreachable!("C PTP output has a valid message kind"),
        }
    }
}

pub(super) fn result(code: u32) -> Result<(), SyncError> {
    match code {
        0 => Ok(()), 1 => Err(SyncError::InvalidNode), 2 => Err(SyncError::InvalidTimestamp),
        3 => Err(SyncError::InvalidPacket), 4 => Err(SyncError::BufferTooSmall { required: PTP_PACKET_BYTES }),
        5 => Err(SyncError::UnexpectedMessage), 6 => Err(SyncError::SequenceMismatch),
        7 => Err(SyncError::NoPendingExchange), 8 => Err(SyncError::ExchangeInProgress),
        9 => Err(SyncError::Unsynchronized), 10 => Err(SyncError::CounterOverflow),
        _ => unreachable!("unknown C time-sync error"),
    }
}

// Only this module and its parent use call; every supplied C function initializes
// its complete output on success. No output is read after an error.
pub(super) fn call<T>(operation: impl FnOnce(*mut T) -> u32) -> Result<T, SyncError> {
    let mut output = MaybeUninit::uninit();
    result(operation(output.as_mut_ptr()))?;
    Ok(unsafe { output.assume_init() })
}

unsafe extern "C" {
    pub(super) fn ghostos_manual_clock_now(clock: *const u64) -> u64;
    pub(super) fn ghostos_manual_clock_set(clock: *const u64, now: u64);
    pub(super) fn ghostos_manual_clock_advance(clock: *const u64, delta: u64);
    pub(super) fn ghostos_time_from_nanos(value: u64, out: *mut PtpTimestamp);
    pub(super) fn ghostos_time_to_nanos(value: *const PtpTimestamp, out: *mut u64) -> u32;
    pub(super) fn ghostos_ptp_encode(message: *const Message, output: *mut u8, capacity: usize) -> u32;
    pub(super) fn ghostos_ptp_decode(bytes: *const u8, length: usize, out: *mut Message) -> u32;
    pub(super) fn ghostos_clock_discipline(clock: *mut ClusterClock, measurement: *const SyncMeasurement,
        now: u64, out: *mut ClockAdjustment);
    pub(super) fn ghostos_clock_corrected_time(clock: *mut ClusterClock, local: u64) -> u64;
    pub(super) fn ghostos_epoch_init(counter: *mut MonotonicEpochCounter, node: u32, initial: u64) -> u32;
    pub(super) fn ghostos_epoch_issue(counter: *mut MonotonicEpochCounter, hardware: u64, out: *mut EpochStamp) -> u32;
    pub(super) fn ghostos_epoch_observe(counter: *mut MonotonicEpochCounter, remote: *const EpochStamp) -> u32;
    pub(super) fn ghostos_ptp_daemon_init(daemon: *mut PtpDaemon, node: u32, role: u8) -> u32;
    pub(super) fn ghostos_ptp_sync(daemon: *mut PtpDaemon, target: u32, tx: u64, two_step: bool, out: *mut Message) -> u32;
    pub(super) fn ghostos_ptp_follow_up(daemon: *const PtpDaemon, sync: *const Message, tx: u64, out: *mut Message) -> u32;
    pub(super) fn ghostos_ptp_receive_sync(daemon: *mut PtpDaemon, message: *const Message, rx: u64) -> u32;
    pub(super) fn ghostos_ptp_receive_follow_up(daemon: *mut PtpDaemon, message: *const Message) -> u32;
    pub(super) fn ghostos_ptp_delay_request(daemon: *mut PtpDaemon, tx: u64, out: *mut Message) -> u32;
    pub(super) fn ghostos_ptp_receive_delay_request(daemon: *const PtpDaemon, message: *const Message, rx: u64, out: *mut Message) -> u32;
    pub(super) fn ghostos_ptp_receive_delay_response(daemon: *mut PtpDaemon, message: *const Message, rx: u64, out: *mut SyncMeasurement) -> u32;
    pub(super) fn ghostos_ptp_synchronized_time(daemon: *mut PtpDaemon, local: u64, out: *mut u64) -> u32;
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<core::sync::atomic::AtomicU64>() == 8);
    assert!(core::mem::align_of::<core::sync::atomic::AtomicU64>() == 8);
    assert!(core::mem::size_of::<PtpTimestamp>() == 16);
    assert!(core::mem::size_of::<Message>() == 56);
    assert!(core::mem::size_of::<Pending>() == 48);
    assert!(core::mem::size_of::<ClusterClock>() == 48);
    assert!(core::mem::size_of::<PtpDaemon>() == 136);
    assert!(core::mem::offset_of!(PtpDaemon, clock) == 64);
    assert!(core::mem::size_of::<MonotonicEpochCounter>() == 24);
};
