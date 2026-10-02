#![no_std]

mod native;

use ghostos_status::{IntoStatus, Severity, Status, facility};

pub const PTP_PACKET_BYTES: usize = 64;
pub const PTP_VERSION: u8 = 2;
pub const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
pub const MAX_FREQUENCY_ADJUSTMENT_PPB: i64 = 500_000;

/// Source of monotonic time for deadline-driven state machines.
///
/// Implementations must never use wall-clock corrections for this value. The
/// clock is deliberately tiny so kernel clocks, VM clocks, and deterministic
/// test clocks can all drive the same lease and timer code.
pub trait MonotonicClock {
    fn now_us(&self) -> u64;
}

/// Deterministic monotonic clock for model tests and replay.
#[derive(Debug)]
pub struct ManualClock {
    now_us: core::sync::atomic::AtomicU64,
}

impl ManualClock {
    pub const fn new(now_us: u64) -> Self {
        Self {
            now_us: core::sync::atomic::AtomicU64::new(now_us),
        }
    }

    pub fn now_us(&self) -> u64 {
        unsafe { native::ghostos_manual_clock_now((&self.now_us as *const core::sync::atomic::AtomicU64).cast()) }
    }

    pub fn set_us(&self, now_us: u64) {
        unsafe { native::ghostos_manual_clock_set((&self.now_us as *const core::sync::atomic::AtomicU64).cast(), now_us); }
    }

    pub fn advance_us(&self, delta_us: u64) {
        unsafe { native::ghostos_manual_clock_advance((&self.now_us as *const core::sync::atomic::AtomicU64).cast(), delta_us); }
    }

}

impl Clone for ManualClock {
    fn clone(&self) -> Self {
        Self::new(self.now_us())
    }
}

impl PartialEq for ManualClock {
    fn eq(&self, other: &Self) -> bool {
        self.now_us() == other.now_us()
    }
}

impl Eq for ManualClock {}

impl MonotonicClock for ManualClock {
    fn now_us(&self) -> u64 {
        self.now_us()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncError {
    InvalidNode,
    InvalidTimestamp,
    InvalidPacket,
    BufferTooSmall { required: usize },
    UnexpectedMessage,
    SequenceMismatch,
    NoPendingExchange,
    ExchangeInProgress,
    Unsynchronized,
    CounterOverflow,
}

impl IntoStatus for SyncError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } | Self::InvalidNode | Self::InvalidTimestamp => {
                Status::INVALID_ARGUMENT
            }
            Self::InvalidPacket => Status::CORRUPT,
            Self::UnexpectedMessage | Self::SequenceMismatch | Self::NoPendingExchange => {
                Status::new(Severity::Warning, facility::NETWORK, 5, 0)
                    .expect("valid time-sync status")
            }
            Self::ExchangeInProgress => Status::BUSY,
            Self::Unsynchronized => Status::PENDING,
            Self::CounterOverflow => Status::new(Severity::Fatal, facility::FABRIC, 5, 0)
                .expect("valid time-sync status"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(C)]
pub struct PtpTimestamp {
    pub seconds: u64,
    pub nanoseconds: u32,
}

impl PtpTimestamp {
    pub const ZERO: Self = Self {
        seconds: 0,
        nanoseconds: 0,
    };

    pub const fn new(seconds: u64, nanoseconds: u32) -> Result<Self, SyncError> {
        if seconds > 0x0000_FFFF_FFFF_FFFF || nanoseconds >= 1_000_000_000 {
            Err(SyncError::InvalidTimestamp)
        } else {
            Ok(Self {
                seconds,
                nanoseconds,
            })
        }
    }

    pub fn from_nanos(value: u64) -> Result<Self, SyncError> {
        native::call(|out| { unsafe { native::ghostos_time_from_nanos(value, out); } 0 })
    }

    pub fn to_nanos(self) -> Result<u64, SyncError> {
        native::call(|out| unsafe { native::ghostos_time_to_nanos(&self, out) })
    }

}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PtpMessageType {
    Sync = 0,
    FollowUp = 8,
    DelayRequest = 1,
    DelayResponse = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PtpMessage {
    Sync {
        sequence_id: u16,
        source: u32,
        target: u32,
        origin_timestamp: PtpTimestamp,
        correction_ns: i64,
    },
    FollowUp {
        sequence_id: u16,
        source: u32,
        target: u32,
        origin_timestamp: PtpTimestamp,
        correction_ns: i64,
    },
    DelayRequest {
        sequence_id: u16,
        source: u32,
        target: u32,
        origin_timestamp: PtpTimestamp,
        correction_ns: i64,
    },
    DelayResponse {
        sequence_id: u16,
        source: u32,
        target: u32,
        request_timestamp: PtpTimestamp,
        receive_timestamp: PtpTimestamp,
        correction_ns: i64,
    },
}

impl PtpMessage {
    pub fn message_type(self) -> PtpMessageType {
        match self {
            Self::Sync { .. } => PtpMessageType::Sync,
            Self::FollowUp { .. } => PtpMessageType::FollowUp,
            Self::DelayRequest { .. } => PtpMessageType::DelayRequest,
            Self::DelayResponse { .. } => PtpMessageType::DelayResponse,
        }
    }

    pub fn sequence_id(self) -> u16 {
        match self {
            Self::Sync { sequence_id, .. }
            | Self::FollowUp { sequence_id, .. }
            | Self::DelayRequest { sequence_id, .. }
            | Self::DelayResponse { sequence_id, .. } => sequence_id,
        }
    }

    pub fn source(self) -> u32 {
        match self {
            Self::Sync { source, .. }
            | Self::FollowUp { source, .. }
            | Self::DelayRequest { source, .. }
            | Self::DelayResponse { source, .. } => source,
        }
    }

    pub fn target(self) -> u32 {
        match self {
            Self::Sync { target, .. }
            | Self::FollowUp { target, .. }
            | Self::DelayRequest { target, .. }
            | Self::DelayResponse { target, .. } => target,
        }
    }

    pub fn correction_ns(self) -> i64 {
        match self {
            Self::Sync { correction_ns, .. }
            | Self::FollowUp { correction_ns, .. }
            | Self::DelayRequest { correction_ns, .. }
            | Self::DelayResponse { correction_ns, .. } => correction_ns,
        }
    }

    pub fn encode(self, destination: &mut [u8]) -> Result<usize, SyncError> {
        let message = native::Message::from(self);
        native::result(unsafe { native::ghostos_ptp_encode(&message, destination.as_mut_ptr(), destination.len()) })?;
        Ok(PTP_PACKET_BYTES)
    }

    pub fn decode(source: &[u8]) -> Result<Self, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_decode(source.as_ptr(), source.len(), out) })
            .map(native::Message::into_message)
    }

}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PtpRole {
    Master,
    Slave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SyncMeasurement {
    pub sequence_id: u16,
    pub master: u32,
    pub offset_ns: i64,
    pub path_delay_ns: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ClockAdjustment {
    pub offset_ns: i64,
    pub frequency_ppb: i64,
    pub stepped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ClockState {
    pub offset_ns: i64,
    pub frequency_ppb: i64,
    pub path_delay_ns: i64,
    pub last_update_ns: u64,
    pub samples: u32,
    pub synchronized: bool,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ClusterClock {
    state: ClockState,
    last_global_ns: u64,
}

impl ClusterClock {
    pub const fn new() -> Self {
        Self {
            state: ClockState {
                offset_ns: 0,
                frequency_ppb: 0,
                path_delay_ns: 0,
                last_update_ns: 0,
                samples: 0,
                synchronized: false,
            },
            last_global_ns: 0,
        }
    }

    pub const fn state(self) -> ClockState {
        self.state
    }

    pub fn discipline(&mut self, measurement: SyncMeasurement, local_now_ns: u64) -> ClockAdjustment {
        native::call(|out| { unsafe { native::ghostos_clock_discipline(self, &measurement, local_now_ns, out); } 0 })
            .expect("clock discipline cannot fail")
    }

    pub fn corrected_time(&mut self, local_ns: u64) -> u64 {
        unsafe { native::ghostos_clock_corrected_time(self, local_ns) }
    }

    pub fn is_synchronized(self) -> bool {
        self.state.synchronized
    }
}

impl Default for ClusterClock {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct PtpDaemon {
    node: u32,
    role: PtpRole,
    next_sequence: u16,
    pending: native::Pending,
    has_pending: bool,
    clock: ClusterClock,
    epoch: MonotonicEpochCounter,
}

impl PtpDaemon {
    pub fn new(node: u32, role: PtpRole) -> Result<Self, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_daemon_init(out, node, role as u8) })
    }

    pub const fn node(&self) -> u32 { self.node }
    pub const fn role(&self) -> PtpRole { self.role }
    pub const fn clock(&self) -> ClockState { self.clock.state() }
    pub fn clock_mut(&mut self) -> &mut ClusterClock { &mut self.clock }

    pub fn sync(&mut self, target: u32, tx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_sync(self, target, tx_timestamp_ns, false, out) })
            .map(native::Message::into_message)
    }

    pub fn sync_two_step(&mut self, target: u32) -> Result<PtpMessage, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_sync(self, target, 0, true, out) })
            .map(native::Message::into_message)
    }

    pub fn follow_up(&self, sync: PtpMessage, tx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        let sync = native::Message::from(sync);
        native::call(|out| unsafe { native::ghostos_ptp_follow_up(self, &sync, tx_timestamp_ns, out) })
            .map(native::Message::into_message)
    }

    pub fn receive_sync(&mut self, message: PtpMessage, rx_timestamp_ns: u64) -> Result<(), SyncError> {
        native::result(unsafe { native::ghostos_ptp_receive_sync(self, &native::Message::from(message), rx_timestamp_ns) })
    }

    pub fn receive_follow_up(&mut self, message: PtpMessage) -> Result<(), SyncError> {
        native::result(unsafe { native::ghostos_ptp_receive_follow_up(self, &native::Message::from(message)) })
    }

    pub fn delay_request(&mut self, tx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_delay_request(self, tx_timestamp_ns, out) })
            .map(native::Message::into_message)
    }

    pub fn receive_delay_request(&self, message: PtpMessage, rx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_receive_delay_request(self, &native::Message::from(message), rx_timestamp_ns, out) })
            .map(native::Message::into_message)
    }

    pub fn receive_delay_response(&mut self, message: PtpMessage, rx_timestamp_ns: u64) -> Result<SyncMeasurement, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_receive_delay_response(self, &native::Message::from(message), rx_timestamp_ns, out) })
    }

    pub fn global_time(&mut self, local_timestamp_ns: u64) -> u64 {
        self.clock.corrected_time(local_timestamp_ns)
    }

    pub fn synchronized_time(&mut self, local_timestamp_ns: u64) -> Result<u64, SyncError> {
        native::call(|out| unsafe { native::ghostos_ptp_synchronized_time(self, local_timestamp_ns, out) })
    }

    pub fn issue_epoch(&mut self, hardware_counter: u64) -> Result<EpochStamp, SyncError> {
        self.epoch.issue(hardware_counter)
    }

    pub fn observe_epoch(&mut self, remote: EpochStamp) -> Result<(), SyncError> { self.epoch.observe(remote) }

    pub fn next_event(&mut self, local_timestamp_ns: u64, hardware_counter: u64) -> Result<GlobalEventTimestamp, SyncError> {
        Ok(GlobalEventTimestamp { time_ns: self.synchronized_time(local_timestamp_ns)?, epoch: self.issue_epoch(hardware_counter)? })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(C)]
pub struct EpochStamp {
    pub epoch: u64,
    pub counter: u64,
    pub node: u32,
}

impl EpochStamp {
    pub const fn is_valid(self) -> bool {
        self.node != 0
    }
}

pub trait HardwareMonotonicCounter {
    fn read(&mut self) -> u64;
}

#[repr(C)]
pub struct MonotonicEpochCounter {
    node: u32,
    epoch: u64,
    counter: u64,
}

impl MonotonicEpochCounter {
    pub fn new(node: u32, initial_counter: u64) -> Result<Self, SyncError> {
        native::call(|out| unsafe { native::ghostos_epoch_init(out, node, initial_counter) })
    }

    pub const fn current(&self) -> EpochStamp {
        EpochStamp {
            epoch: self.epoch,
            counter: self.counter,
            node: self.node,
        }
    }

    pub fn issue(&mut self, hardware_counter: u64) -> Result<EpochStamp, SyncError> {
        native::call(|out| unsafe { native::ghostos_epoch_issue(self, hardware_counter, out) })
    }

    pub fn issue_from<C: HardwareMonotonicCounter>(
        &mut self,
        source: &mut C,
    ) -> Result<EpochStamp, SyncError> {
        self.issue(source.read())
    }

    pub fn observe(&mut self, remote: EpochStamp) -> Result<(), SyncError> {
        native::result(unsafe { native::ghostos_epoch_observe(self, &remote) })
    }

}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct GlobalEventTimestamp {
    pub time_ns: u64,
    pub epoch: EpochStamp,
}
