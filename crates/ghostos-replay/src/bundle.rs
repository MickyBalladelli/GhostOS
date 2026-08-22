//! Portable, bounded replay bundles.
//!
//! The bundle contains only deterministic inputs. Configuration is represented
//! by a digest, never by the configuration bytes themselves. Payload-bearing
//! records require an explicit public sensitivity marker, so secret data is
//! rejected before it can be serialized.

use super::ReplayError;

pub const REPLAY_BUNDLE_FORMAT_VERSION: u16 = 1;
pub const REPLAY_BUNDLE_CONFIG_DIGEST_BYTES: usize = 32;
pub const REPLAY_BUNDLE_MAX_EVENTS: usize = 128;
pub const REPLAY_BUNDLE_MAX_PAYLOAD_BYTES: usize = 256;
pub const REPLAY_BUNDLE_HEADER_BYTES: usize = 64;
pub const REPLAY_BUNDLE_EVENT_BYTES: usize = 58 + REPLAY_BUNDLE_MAX_PAYLOAD_BYTES;

const REPLAY_BUNDLE_MAGIC: [u8; 8] = *b"SYNBNDL1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySensitivity {
    Public,
    Secret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReplayBundleEventKind {
    Input = 1,
    Clock = 2,
    RandomSeed = 3,
    DeviceCompletion = 4,
    SchedulerDecision = 5,
}

impl ReplayBundleEventKind {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Input),
            2 => Some(Self::Clock),
            3 => Some(Self::RandomSeed),
            4 => Some(Self::DeviceCompletion),
            5 => Some(Self::SchedulerDecision),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayBundleEvent {
    pub sequence: u64,
    pub timestamp_us: u64,
    pub kind: ReplayBundleEventKind,
    pub source: u64,
    pub value0: u64,
    pub value1: u64,
    pub value2: u64,
    payload_len: u16,
    payload: [u8; REPLAY_BUNDLE_MAX_PAYLOAD_BYTES],
}

impl ReplayBundleEvent {
    pub const EMPTY: Self = Self {
        sequence: 0,
        timestamp_us: 0,
        kind: ReplayBundleEventKind::Input,
        source: 0,
        value0: 0,
        value1: 0,
        value2: 0,
        payload_len: 0,
        payload: [0; REPLAY_BUNDLE_MAX_PAYLOAD_BYTES],
    };

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }

    pub const fn payload_len(&self) -> usize {
        self.payload_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayBundle {
    config_digest: [u8; REPLAY_BUNDLE_CONFIG_DIGEST_BYTES],
    random_seed: u64,
    initial_clock_ns: u64,
    event_count: u16,
    events: [ReplayBundleEvent; REPLAY_BUNDLE_MAX_EVENTS],
}

impl ReplayBundle {
    pub fn new(
        config_digest: [u8; REPLAY_BUNDLE_CONFIG_DIGEST_BYTES],
        random_seed: u64,
        initial_clock_ns: u64,
    ) -> Self {
        Self {
            config_digest,
            random_seed,
            initial_clock_ns,
            event_count: 0,
            events: [ReplayBundleEvent::EMPTY; REPLAY_BUNDLE_MAX_EVENTS],
        }
    }

    pub fn config_digest(&self) -> &[u8; REPLAY_BUNDLE_CONFIG_DIGEST_BYTES] {
        &self.config_digest
    }

    pub const fn random_seed(&self) -> u64 {
        self.random_seed
    }

    pub const fn initial_clock_ns(&self) -> u64 {
        self.initial_clock_ns
    }

    pub fn events(&self) -> &[ReplayBundleEvent] {
        &self.events[..self.event_count as usize]
    }

    pub const fn len(&self) -> usize {
        self.event_count as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.event_count == 0
    }

    pub const fn encoded_len(&self) -> usize {
        REPLAY_BUNDLE_HEADER_BYTES + self.event_count as usize * REPLAY_BUNDLE_EVENT_BYTES
    }

    pub fn record_input(
        &mut self,
        timestamp_us: u64,
        source: u64,
        input: &[u8],
        sensitivity: ReplaySensitivity,
    ) -> Result<(), ReplayError> {
        if sensitivity == ReplaySensitivity::Secret {
            return Err(ReplayError::SecretExcluded)
        }
        self.append(
            ReplayBundleEventKind::Input,
            timestamp_us,
            source,
            public_input_digest(input),
            input.len() as u64,
            0,
            input,
        )
    }

    pub fn record_clock(&mut self, timestamp_us: u64, clock_ns: u64) -> Result<(), ReplayError> {
        self.append(
            ReplayBundleEventKind::Clock,
            timestamp_us,
            0,
            clock_ns,
            0,
            0,
            &[],
        )
    }

    pub fn record_random_seed(
        &mut self,
        timestamp_us: u64,
        source: u64,
        seed: u64,
    ) -> Result<(), ReplayError> {
        self.append(
            ReplayBundleEventKind::RandomSeed,
            timestamp_us,
            source,
            seed,
            0,
            0,
            &[],
        )
    }

    pub fn record_device_completion(
        &mut self,
        timestamp_us: u64,
        device: u64,
        completion_code: u64,
        data: &[u8],
        sensitivity: ReplaySensitivity,
    ) -> Result<(), ReplayError> {
        if sensitivity == ReplaySensitivity::Secret {
            return Err(ReplayError::SecretExcluded)
        }
        self.append(
            ReplayBundleEventKind::DeviceCompletion,
            timestamp_us,
            device,
            completion_code,
            data.len() as u64,
            public_input_digest(data),
            data,
        )
    }

    pub fn record_scheduler_decision(
        &mut self,
        timestamp_us: u64,
        cpu: u64,
        runnable_set: u64,
        selected_thread: u64,
    ) -> Result<(), ReplayError> {
        self.append(
            ReplayBundleEventKind::SchedulerDecision,
            timestamp_us,
            cpu,
            runnable_set,
            selected_thread,
            0,
            &[],
        )
    }

    pub fn replay(&self) -> ReplayBundleReader<'_> {
        ReplayBundleReader {
            bundle: self,
            cursor: 0,
        }
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ReplayError> {
        let required = self.encoded_len();
        if output.len() < required {
            return Err(ReplayError::BufferTooSmall { required })
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(&REPLAY_BUNDLE_MAGIC);
        output[8..10].copy_from_slice(&REPLAY_BUNDLE_FORMAT_VERSION.to_le_bytes());
        output[12..16].copy_from_slice(&(self.event_count as u32).to_le_bytes());
        output[16..48].copy_from_slice(&self.config_digest);
        output[48..56].copy_from_slice(&self.random_seed.to_le_bytes());
        output[56..64].copy_from_slice(&self.initial_clock_ns.to_le_bytes());

        for (index, event) in self.events().iter().enumerate() {
            let start = REPLAY_BUNDLE_HEADER_BYTES + index * REPLAY_BUNDLE_EVENT_BYTES;
            encode_event(event, &mut output[start..start + REPLAY_BUNDLE_EVENT_BYTES]);
        }
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, ReplayError> {
        if input.len() < REPLAY_BUNDLE_HEADER_BYTES || input[..8] != REPLAY_BUNDLE_MAGIC {
            return Err(ReplayError::Corrupt)
        }
        if read_u16(&input[8..10]) != REPLAY_BUNDLE_FORMAT_VERSION {
            return Err(ReplayError::UnsupportedVersion)
        }
        if read_u16(&input[10..12]) != 0 {
            return Err(ReplayError::Corrupt)
        }
        let event_count = read_u32(&input[12..16]) as usize;
        if event_count > REPLAY_BUNDLE_MAX_EVENTS {
            return Err(ReplayError::Capacity)
        }
        let required = REPLAY_BUNDLE_HEADER_BYTES
            .checked_add(event_count.checked_mul(REPLAY_BUNDLE_EVENT_BYTES).ok_or(ReplayError::Corrupt)?)
            .ok_or(ReplayError::Corrupt)?;
        if input.len() != required {
            return Err(ReplayError::Corrupt)
        }

        let mut config_digest = [0; REPLAY_BUNDLE_CONFIG_DIGEST_BYTES];
        config_digest.copy_from_slice(&input[16..48]);
        let mut bundle = Self::new(
            config_digest,
            read_u64(&input[48..56]),
            read_u64(&input[56..64]),
        );
        for index in 0..event_count {
            let start = REPLAY_BUNDLE_HEADER_BYTES + index * REPLAY_BUNDLE_EVENT_BYTES;
            let event = decode_event(&input[start..start + REPLAY_BUNDLE_EVENT_BYTES])?;
            if event.sequence != index as u64 + 1 {
                return Err(ReplayError::Corrupt)
            }
            bundle.events[index] = event;
        }
        bundle.event_count = event_count as u16;
        Ok(bundle)
    }

    fn append(
        &mut self,
        kind: ReplayBundleEventKind,
        timestamp_us: u64,
        source: u64,
        value0: u64,
        value1: u64,
        value2: u64,
        payload: &[u8],
    ) -> Result<(), ReplayError> {
        if payload.len() > REPLAY_BUNDLE_MAX_PAYLOAD_BYTES {
            return Err(ReplayError::BufferTooSmall {
                required: payload.len(),
            })
        }
        let index = self.event_count as usize;
        if index == REPLAY_BUNDLE_MAX_EVENTS {
            return Err(ReplayError::Capacity)
        }
        let mut event = ReplayBundleEvent::EMPTY;
        event.sequence = index as u64 + 1;
        event.timestamp_us = timestamp_us;
        event.kind = kind;
        event.source = source;
        event.value0 = value0;
        event.value1 = value1;
        event.value2 = value2;
        event.payload_len = payload.len() as u16;
        event.payload[..payload.len()].copy_from_slice(payload);
        self.events[index] = event;
        self.event_count += 1;
        Ok(())
    }
}

pub struct ReplayBundleReader<'a> {
    bundle: &'a ReplayBundle,
    cursor: usize,
}

impl<'a> ReplayBundleReader<'a> {
    pub const fn position(&self) -> usize {
        self.cursor
    }

    pub const fn pending(&self) -> usize {
        self.bundle.len().saturating_sub(self.cursor)
    }

    pub fn next(
        &mut self,
        expected: ReplayBundleEventKind,
    ) -> Result<ReplayBundleEvent, ReplayError> {
        let event = *self
            .bundle
            .events()
            .get(self.cursor)
            .ok_or(ReplayError::NoEvent)?;
        if event.kind != expected {
            return Err(ReplayError::UnexpectedEvent)
        }
        self.cursor += 1;
        Ok(event)
    }

    pub fn input(&mut self, source: u64, actual: &[u8]) -> Result<(), ReplayError> {
        let event = self.next(ReplayBundleEventKind::Input)?;
        if event.source != source
            || event.payload() != actual
            || event.value0 != public_input_digest(actual)
        {
            return Err(ReplayError::InputMismatch)
        }
        Ok(())
    }

    pub fn next_input(&mut self, source: u64) -> Result<ReplayBundleEvent, ReplayError> {
        let event = self.next(ReplayBundleEventKind::Input)?;
        if event.source != source {
            return Err(ReplayError::InputMismatch)
        }
        Ok(event)
    }

    pub fn clock(&mut self) -> Result<u64, ReplayError> {
        Ok(self.next(ReplayBundleEventKind::Clock)?.value0)
    }

    pub fn random_seed(&mut self, source: u64) -> Result<u64, ReplayError> {
        let event = self.next(ReplayBundleEventKind::RandomSeed)?;
        if event.source != source {
            return Err(ReplayError::InputMismatch)
        }
        Ok(event.value0)
    }

    pub fn device_completion(
        &mut self,
        device: u64,
    ) -> Result<ReplayBundleEvent, ReplayError> {
        let event = self.next(ReplayBundleEventKind::DeviceCompletion)?;
        if event.source != device {
            return Err(ReplayError::InputMismatch)
        }
        Ok(event)
    }

    pub fn scheduler_decision(&mut self, cpu: u64) -> Result<(u64, u64), ReplayError> {
        let event = self.next(ReplayBundleEventKind::SchedulerDecision)?;
        if event.source != cpu {
            return Err(ReplayError::InputMismatch)
        }
        Ok((event.value0, event.value1))
    }
}

pub fn public_input_digest(input: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    hash
}

fn encode_event(event: &ReplayBundleEvent, output: &mut [u8]) {
    output[..8].copy_from_slice(&event.sequence.to_le_bytes());
    output[8..16].copy_from_slice(&event.timestamp_us.to_le_bytes());
    output[16] = event.kind as u8;
    output[24..32].copy_from_slice(&event.source.to_le_bytes());
    output[32..40].copy_from_slice(&event.value0.to_le_bytes());
    output[40..48].copy_from_slice(&event.value1.to_le_bytes());
    output[48..56].copy_from_slice(&event.value2.to_le_bytes());
    output[56..58].copy_from_slice(&event.payload_len.to_le_bytes());
    output[58..58 + REPLAY_BUNDLE_MAX_PAYLOAD_BYTES].copy_from_slice(&event.payload);
}

fn decode_event(input: &[u8]) -> Result<ReplayBundleEvent, ReplayError> {
    let kind = ReplayBundleEventKind::from_raw(input[16]).ok_or(ReplayError::Corrupt)?;
    if input[17..24].iter().any(|byte| *byte != 0) {
        return Err(ReplayError::Corrupt)
    }
    let payload_len = read_u16(&input[56..58]) as usize;
    if payload_len > REPLAY_BUNDLE_MAX_PAYLOAD_BYTES {
        return Err(ReplayError::Corrupt)
    }
    let mut payload = [0; REPLAY_BUNDLE_MAX_PAYLOAD_BYTES];
    payload.copy_from_slice(&input[58..58 + REPLAY_BUNDLE_MAX_PAYLOAD_BYTES]);
    if payload[payload_len..].iter().any(|byte| *byte != 0) {
        return Err(ReplayError::Corrupt)
    }
    Ok(ReplayBundleEvent {
        sequence: read_u64(&input[..8]),
        timestamp_us: read_u64(&input[8..16]),
        kind,
        source: read_u64(&input[24..32]),
        value0: read_u64(&input[32..40]),
        value1: read_u64(&input[40..48]),
        value2: read_u64(&input[48..56]),
        payload_len: payload_len as u16,
        payload,
    })
}

fn read_u16(input: &[u8]) -> u16 {
    u16::from_le_bytes([input[0], input[1]])
}

fn read_u32(input: &[u8]) -> u32 {
    u32::from_le_bytes([input[0], input[1], input[2], input[3]])
}

fn read_u64(input: &[u8]) -> u64 {
    u64::from_le_bytes([
        input[0], input[1], input[2], input[3], input[4], input[5], input[6], input[7],
    ])
}
