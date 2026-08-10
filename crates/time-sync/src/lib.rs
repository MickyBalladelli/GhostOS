#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

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
        self.now_us.load(core::sync::atomic::Ordering::Acquire)
    }

    pub fn set_us(&self, now_us: u64) {
        self.now_us
            .fetch_max(now_us, core::sync::atomic::Ordering::AcqRel);
    }

    pub fn advance_us(&self, delta_us: u64) {
        let _ = self.now_us.fetch_update(
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
            |now| Some(now.saturating_add(delta_us)),
        );
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

const PTP_MAGIC: [u8; 4] = *b"SPTP";

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
        Self::new(value / NANOSECONDS_PER_SECOND, (value % NANOSECONDS_PER_SECOND) as u32)
    }

    pub fn to_nanos(self) -> Result<u64, SyncError> {
        self.seconds
            .checked_mul(NANOSECONDS_PER_SECOND)
            .and_then(|value| value.checked_add(self.nanoseconds as u64))
            .ok_or(SyncError::InvalidTimestamp)
    }

    fn encode(self, destination: &mut [u8]) -> Result<(), SyncError> {
        if destination.len() < 10 {
            return Err(SyncError::BufferTooSmall { required: 10 })
        }
        let seconds = self.seconds.to_be_bytes();
        destination[..6].copy_from_slice(&seconds[2..]);
        destination[6..10].copy_from_slice(&self.nanoseconds.to_be_bytes());
        Ok(())
    }

    fn decode(source: &[u8]) -> Result<Self, SyncError> {
        if source.len() < 10 {
            return Err(SyncError::InvalidPacket)
        }
        let mut seconds = [0; 8];
        seconds[2..].copy_from_slice(&source[..6]);
        Self::new(
            u64::from_be_bytes(seconds),
            u32::from_be_bytes([source[6], source[7], source[8], source[9]]),
        )
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

impl PtpMessageType {
    fn from_raw(value: u8) -> Result<Self, SyncError> {
        match value {
            0 => Ok(Self::Sync),
            8 => Ok(Self::FollowUp),
            1 => Ok(Self::DelayRequest),
            9 => Ok(Self::DelayResponse),
            _ => Err(SyncError::InvalidPacket),
        }
    }
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
        if destination.len() < PTP_PACKET_BYTES {
            return Err(SyncError::BufferTooSmall {
                required: PTP_PACKET_BYTES,
            })
        }
        if self.source() == 0 || self.target() == 0 {
            return Err(SyncError::InvalidNode)
        }
        destination[..PTP_PACKET_BYTES].fill(0);
        destination[..4].copy_from_slice(&PTP_MAGIC);
        destination[4] = PTP_VERSION;
        destination[5] = self.message_type() as u8;
        destination[8..10].copy_from_slice(&self.sequence_id().to_be_bytes());
        destination[10..14].copy_from_slice(&self.source().to_be_bytes());
        destination[14..18].copy_from_slice(&self.target().to_be_bytes());
        destination[18..26].copy_from_slice(&self.correction_ns().to_be_bytes());
        match self {
            Self::Sync {
                origin_timestamp, ..
            }
            | Self::FollowUp {
                origin_timestamp, ..
            }
            | Self::DelayRequest {
                origin_timestamp, ..
            } => origin_timestamp.encode(&mut destination[26..36])?,
            Self::DelayResponse {
                request_timestamp,
                receive_timestamp,
                ..
            } => {
                request_timestamp.encode(&mut destination[26..36])?;
                receive_timestamp.encode(&mut destination[36..46])?;
            }
        }
        Ok(PTP_PACKET_BYTES)
    }

    pub fn decode(source: &[u8]) -> Result<Self, SyncError> {
        if source.len() < PTP_PACKET_BYTES
            || source[..4] != PTP_MAGIC
            || source[4] != PTP_VERSION
            || source[6] != 0
            || source[7] != 0
        {
            return Err(SyncError::InvalidPacket)
        }
        let message_type = PtpMessageType::from_raw(source[5])?;
        let sequence_id = u16::from_be_bytes([source[8], source[9]]);
        let source_node = u32::from_be_bytes([source[10], source[11], source[12], source[13]]);
        let target = u32::from_be_bytes([source[14], source[15], source[16], source[17]]);
        if source_node == 0 || target == 0 {
            return Err(SyncError::InvalidPacket)
        }
        let correction_ns = i64::from_be_bytes([
            source[18], source[19], source[20], source[21], source[22], source[23], source[24],
            source[25],
        ]);
        let first = PtpTimestamp::decode(&source[26..36])?;
        Ok(match message_type {
            PtpMessageType::Sync => Self::Sync {
                sequence_id,
                source: source_node,
                target,
                origin_timestamp: first,
                correction_ns,
            },
            PtpMessageType::FollowUp => Self::FollowUp {
                sequence_id,
                source: source_node,
                target,
                origin_timestamp: first,
                correction_ns,
            },
            PtpMessageType::DelayRequest => Self::DelayRequest {
                sequence_id,
                source: source_node,
                target,
                origin_timestamp: first,
                correction_ns,
            },
            PtpMessageType::DelayResponse => Self::DelayResponse {
                sequence_id,
                source: source_node,
                target,
                request_timestamp: first,
                receive_timestamp: PtpTimestamp::decode(&source[36..46])?,
                correction_ns,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PtpRole {
    Master,
    Slave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncMeasurement {
    pub sequence_id: u16,
    pub master: u32,
    pub offset_ns: i64,
    pub path_delay_ns: i64,
}

impl SyncMeasurement {
    fn calculate(
        sequence_id: u16,
        master: u32,
        t1: u64,
        t2: u64,
        t3: u64,
        t4: u64,
        correction_ns: i64,
    ) -> Self {
        let master_to_slave = t2 as i128 - t1 as i128;
        let slave_to_master = t4 as i128 - t3 as i128;
        let correction = correction_ns as i128;
        let offset = (master_to_slave - slave_to_master - correction) / 2;
        let delay = (master_to_slave + slave_to_master - correction) / 2;
        Self {
            sequence_id,
            master,
            offset_ns: clamp_i128(offset),
            path_delay_ns: clamp_i128(delay),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockAdjustment {
    pub offset_ns: i64,
    pub frequency_ppb: i64,
    pub stepped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockState {
    pub offset_ns: i64,
    pub frequency_ppb: i64,
    pub path_delay_ns: i64,
    pub last_update_ns: u64,
    pub samples: u32,
    pub synchronized: bool,
}

#[derive(Clone, Copy, Debug)]
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

    pub fn discipline(
        &mut self,
        measurement: SyncMeasurement,
        local_now_ns: u64,
    ) -> ClockAdjustment {
        let previous_offset = self.state.offset_ns;
        let first_sample = self.state.samples == 0;
        let offset = if first_sample {
            measurement.offset_ns
        } else {
            previous_offset.saturating_add(measurement.offset_ns.saturating_sub(previous_offset) / 4)
        };
        let frequency_ppb = if first_sample || local_now_ns <= self.state.last_update_ns {
            0
        } else {
            let elapsed = local_now_ns - self.state.last_update_ns;
            let delta = offset.saturating_sub(previous_offset) as i128;
            clamp_i128((delta * 1_000_000_000_i128) / elapsed as i128)
                .clamp(-MAX_FREQUENCY_ADJUSTMENT_PPB, MAX_FREQUENCY_ADJUSTMENT_PPB)
        };
        let stepped = first_sample || measurement.offset_ns.unsigned_abs() > 1_000_000;
        self.state = ClockState {
            offset_ns: offset,
            frequency_ppb,
            path_delay_ns: measurement.path_delay_ns,
            last_update_ns: local_now_ns,
            samples: self.state.samples.saturating_add(1),
            synchronized: true,
        };
        ClockAdjustment {
            offset_ns: offset,
            frequency_ppb,
            stepped,
        }
    }

    pub fn corrected_time(&mut self, local_ns: u64) -> u64 {
        let elapsed = local_ns.saturating_sub(self.state.last_update_ns) as i128;
        let frequency_correction = elapsed * self.state.frequency_ppb as i128 / 1_000_000_000_i128;
        let corrected = local_ns as i128 + self.state.offset_ns as i128 + frequency_correction;
        let corrected = corrected.clamp(0, u64::MAX as i128) as u64;
        let monotonic = corrected.max(self.last_global_ns.saturating_add(1));
        self.last_global_ns = monotonic;
        monotonic
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

#[derive(Clone, Copy)]
struct PendingExchange {
    sequence_id: u16,
    master: u32,
    t1: Option<u64>,
    t2: Option<u64>,
    t3: Option<u64>,
    correction_ns: i64,
}

pub struct PtpDaemon {
    node: u32,
    role: PtpRole,
    next_sequence: u16,
    pending: Option<PendingExchange>,
    clock: ClusterClock,
    epoch: MonotonicEpochCounter,
}

impl PtpDaemon {
    pub fn new(node: u32, role: PtpRole) -> Result<Self, SyncError> {
        if node == 0 {
            return Err(SyncError::InvalidNode)
        }
        Ok(Self {
            node,
            role,
            next_sequence: 1,
            pending: None,
            clock: ClusterClock::new(),
            epoch: MonotonicEpochCounter::new(node, 0)?,
        })
    }

    pub const fn node(&self) -> u32 {
        self.node
    }

    pub const fn role(&self) -> PtpRole {
        self.role
    }

    pub const fn clock(&self) -> ClockState {
        self.clock.state()
    }

    pub fn clock_mut(&mut self) -> &mut ClusterClock {
        &mut self.clock
    }

    pub fn sync(&mut self, target: u32, tx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        self.check_peer(target)?;
        if self.role != PtpRole::Master {
            return Err(SyncError::UnexpectedMessage)
        }
        let sequence_id = self.take_sequence();
        Ok(PtpMessage::Sync {
            sequence_id,
            source: self.node,
            target,
            origin_timestamp: PtpTimestamp::from_nanos(tx_timestamp_ns)?,
            correction_ns: 0,
        })
    }

    pub fn sync_two_step(&mut self, target: u32) -> Result<PtpMessage, SyncError> {
        self.check_peer(target)?;
        if self.role != PtpRole::Master {
            return Err(SyncError::UnexpectedMessage)
        }
        Ok(PtpMessage::Sync {
            sequence_id: self.take_sequence(),
            source: self.node,
            target,
            origin_timestamp: PtpTimestamp::ZERO,
            correction_ns: 0,
        })
    }

    pub fn follow_up(
        &self,
        sync: PtpMessage,
        tx_timestamp_ns: u64,
    ) -> Result<PtpMessage, SyncError> {
        let PtpMessage::Sync {
            sequence_id,
            source,
            target,
            origin_timestamp,
            correction_ns,
        } = sync
        else {
            return Err(SyncError::UnexpectedMessage)
        };
        if source != self.node || origin_timestamp != PtpTimestamp::ZERO {
            return Err(SyncError::UnexpectedMessage)
        }
        Ok(PtpMessage::FollowUp {
            sequence_id,
            source,
            target,
            origin_timestamp: PtpTimestamp::from_nanos(tx_timestamp_ns)?,
            correction_ns,
        })
    }

    pub fn receive_sync(
        &mut self,
        message: PtpMessage,
        rx_timestamp_ns: u64,
    ) -> Result<(), SyncError> {
        let PtpMessage::Sync {
            sequence_id,
            source,
            target,
            origin_timestamp,
            correction_ns,
        } = message
        else {
            return Err(SyncError::UnexpectedMessage)
        };
        self.check_target(target)?;
        if self.role != PtpRole::Slave || source == self.node {
            return Err(SyncError::UnexpectedMessage)
        }
        if self.pending.is_some_and(|entry| entry.t3.is_some()) {
            return Err(SyncError::ExchangeInProgress)
        }
        let t1 = if origin_timestamp == PtpTimestamp::ZERO {
            None
        } else {
            Some(origin_timestamp.to_nanos()?)
        };
        self.pending = Some(PendingExchange {
            sequence_id,
            master: source,
            t1,
            t2: Some(rx_timestamp_ns),
            t3: None,
            correction_ns,
        });
        Ok(())
    }

    pub fn receive_follow_up(
        &mut self,
        message: PtpMessage,
    ) -> Result<(), SyncError> {
        let PtpMessage::FollowUp {
            sequence_id,
            source,
            target,
            origin_timestamp,
            correction_ns,
        } = message
        else {
            return Err(SyncError::UnexpectedMessage)
        };
        self.check_target(target)?;
        let pending = self.pending.as_mut().ok_or(SyncError::NoPendingExchange)?;
        if pending.sequence_id != sequence_id || pending.master != source {
            return Err(SyncError::SequenceMismatch)
        }
        pending.t1 = Some(origin_timestamp.to_nanos()?);
        pending.correction_ns = correction_ns;
        Ok(())
    }

    pub fn delay_request(&mut self, tx_timestamp_ns: u64) -> Result<PtpMessage, SyncError> {
        if self.role != PtpRole::Slave {
            return Err(SyncError::UnexpectedMessage)
        }
        let pending = self.pending.as_mut().ok_or(SyncError::NoPendingExchange)?;
        if pending.t1.is_none() {
            return Err(SyncError::NoPendingExchange)
        }
        pending.t3 = Some(tx_timestamp_ns);
        Ok(PtpMessage::DelayRequest {
            sequence_id: pending.sequence_id,
            source: self.node,
            target: pending.master,
            origin_timestamp: PtpTimestamp::from_nanos(tx_timestamp_ns)?,
            correction_ns: 0,
        })
    }

    pub fn receive_delay_request(
        &self,
        message: PtpMessage,
        rx_timestamp_ns: u64,
    ) -> Result<PtpMessage, SyncError> {
        let PtpMessage::DelayRequest {
            sequence_id,
            source,
            target,
            origin_timestamp,
            correction_ns,
        } = message
        else {
            return Err(SyncError::UnexpectedMessage)
        };
        self.check_target(target)?;
        if self.role != PtpRole::Master || source == self.node {
            return Err(SyncError::UnexpectedMessage)
        }
        Ok(PtpMessage::DelayResponse {
            sequence_id,
            source: self.node,
            target: source,
            request_timestamp: origin_timestamp,
            receive_timestamp: PtpTimestamp::from_nanos(rx_timestamp_ns)?,
            correction_ns,
        })
    }

    pub fn receive_delay_response(
        &mut self,
        message: PtpMessage,
        rx_timestamp_ns: u64,
    ) -> Result<SyncMeasurement, SyncError> {
        let PtpMessage::DelayResponse {
            sequence_id,
            source,
            target,
            request_timestamp,
            receive_timestamp,
            correction_ns,
        } = message
        else {
            return Err(SyncError::UnexpectedMessage)
        };
        self.check_target(target)?;
        if self.role != PtpRole::Slave || source == self.node {
            return Err(SyncError::UnexpectedMessage)
        }
        let pending = self.pending.ok_or(SyncError::NoPendingExchange)?;
        if pending.sequence_id != sequence_id || pending.master != source {
            return Err(SyncError::SequenceMismatch)
        }
        let t1 = pending.t1.ok_or(SyncError::NoPendingExchange)?;
        let t2 = pending.t2.ok_or(SyncError::NoPendingExchange)?;
        let t3 = pending.t3.ok_or(SyncError::NoPendingExchange)?;
        if request_timestamp.to_nanos()? != t3 {
            return Err(SyncError::SequenceMismatch)
        }
        self.pending = None;
        let t4 = receive_timestamp.to_nanos()?;
        let measurement = SyncMeasurement::calculate(
            sequence_id,
            source,
            t1,
            t2,
            t3,
            t4,
            pending.correction_ns.saturating_add(correction_ns),
        );
        self.clock.discipline(measurement, rx_timestamp_ns);
        Ok(measurement)
    }

    pub fn global_time(&mut self, local_timestamp_ns: u64) -> u64 {
        self.clock.corrected_time(local_timestamp_ns)
    }

    pub fn synchronized_time(&mut self, local_timestamp_ns: u64) -> Result<u64, SyncError> {
        if self.role == PtpRole::Slave && !self.clock.is_synchronized() {
            return Err(SyncError::Unsynchronized)
        }
        Ok(self.global_time(local_timestamp_ns))
    }

    pub fn issue_epoch(&mut self, hardware_counter: u64) -> Result<EpochStamp, SyncError> {
        self.epoch.issue(hardware_counter)
    }

    pub fn observe_epoch(&mut self, remote: EpochStamp) -> Result<(), SyncError> {
        self.epoch.observe(remote)
    }

    pub fn next_event(
        &mut self,
        local_timestamp_ns: u64,
        hardware_counter: u64,
    ) -> Result<GlobalEventTimestamp, SyncError> {
        Ok(GlobalEventTimestamp {
            time_ns: self.synchronized_time(local_timestamp_ns)?,
            epoch: self.issue_epoch(hardware_counter)?,
        })
    }

    fn take_sequence(&mut self) -> u16 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        sequence
    }

    fn check_peer(&self, node: u32) -> Result<(), SyncError> {
        if node == 0 || node == self.node {
            Err(SyncError::InvalidNode)
        } else {
            Ok(())
        }
    }

    fn check_target(&self, target: u32) -> Result<(), SyncError> {
        if target == self.node {
            Ok(())
        } else {
            Err(SyncError::UnexpectedMessage)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
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

pub struct MonotonicEpochCounter {
    node: u32,
    epoch: u64,
    counter: u64,
}

impl MonotonicEpochCounter {
    pub fn new(node: u32, initial_counter: u64) -> Result<Self, SyncError> {
        if node == 0 {
            return Err(SyncError::InvalidNode)
        }
        Ok(Self {
            node,
            epoch: 1,
            counter: initial_counter,
        })
    }

    pub const fn current(&self) -> EpochStamp {
        EpochStamp {
            epoch: self.epoch,
            counter: self.counter,
            node: self.node,
        }
    }

    pub fn issue(&mut self, hardware_counter: u64) -> Result<EpochStamp, SyncError> {
        let floor = self.counter.max(hardware_counter);
        self.counter = floor.checked_add(1).ok_or(SyncError::CounterOverflow)?;
        Ok(self.current())
    }

    pub fn issue_from<C: HardwareMonotonicCounter>(
        &mut self,
        source: &mut C,
    ) -> Result<EpochStamp, SyncError> {
        self.issue(source.read())
    }

    pub fn observe(&mut self, remote: EpochStamp) -> Result<(), SyncError> {
        if !remote.is_valid() {
            return Err(SyncError::InvalidNode)
        }
        if remote.epoch > self.epoch {
            self.epoch = remote.epoch;
            self.counter = remote.counter;
        } else if remote.epoch == self.epoch {
            self.counter = self.counter.max(remote.counter);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct GlobalEventTimestamp {
    pub time_ns: u64,
    pub epoch: EpochStamp,
}

fn clamp_i128(value: i128) -> i64 {
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}
