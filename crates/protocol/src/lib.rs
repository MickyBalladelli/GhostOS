#![no_std]
#![forbid(unsafe_code)]

//! Shared wire-boundary policy for GhostOS transports.
//!
//! The guard is deliberately transport-neutral. HTTP, gRPC, the SDK, remote
//! terminal, mesh, and cluster code provide framing and authentication, while
//! this crate gives each one the same bounded rules for compatibility,
//! replay, flow control, and reconnects.

pub const CURRENT_PROTOCOL_VERSION: u16 = ghostos_abi::ABI_SCHEMA_VERSION;
pub const WIRE_API_VERSION: ghostos_api_compat::ApiVersion = ghostos_api_compat::WIRE_API.current;
pub const REPLAY_WINDOW_BITS: u8 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TrafficClass {
    Http = 1,
    Grpc = 2,
    Sdk = 3,
    RemoteTerminal = 4,
    Mesh = 5,
    Cluster = 6,
}

impl TrafficClass {
    pub const fn limits(self) -> ProtocolLimits {
        match self {
            Self::Http => ProtocolLimits::new(64 * 1024, 64 * 1024, 32, 3, 250_000),
            Self::Grpc => ProtocolLimits::new(1024 * 1024, 1024 * 1024, 64, 3, 1_000_000),
            Self::Sdk => ProtocolLimits::new(4096, 4096, 64, 3, 16_384),
            Self::RemoteTerminal => ProtocolLimits::new(16 * 1024, 16 * 1024, 32, 3, 64 * 1024),
            Self::Mesh => ProtocolLimits::new(4096, 4096, 64, 3, 64 * 1024),
            Self::Cluster => ProtocolLimits::new(4096, 4096, 64, 3, 64 * 1024),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolLimits {
    pub max_message_bytes: usize,
    pub max_inflight_bytes: usize,
    pub max_inflight_messages: u16,
    pub max_auth_failures: u8,
    pub reconnect_base_delay_us: u64,
}

impl ProtocolLimits {
    const fn new(
        max_message_bytes: usize,
        max_inflight_bytes: usize,
        max_inflight_messages: u16,
        max_auth_failures: u8,
        reconnect_base_delay_us: u64,
    ) -> Self {
        Self {
            max_message_bytes,
            max_inflight_bytes,
            max_inflight_messages,
            max_auth_failures,
            reconnect_base_delay_us,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRange {
    pub minimum: u16,
    pub maximum: u16,
}

impl VersionRange {
    pub const fn new(minimum: u16, maximum: u16) -> Option<Self> {
        if minimum == 0 || minimum > maximum {
            None
        } else {
            Some(Self { minimum, maximum })
        }
    }

    pub const fn contains(self, version: u16) -> bool {
        version >= self.minimum && version <= self.maximum
    }
}

pub const fn negotiate_versions(
    local: VersionRange,
    peer: VersionRange,
) -> Result<u16, ProtocolError> {
    if local.minimum == 0
        || local.minimum > local.maximum
        || peer.minimum == 0
        || peer.minimum > peer.maximum
    {
        return Err(ProtocolError::InvalidVersionRange)
    }
    let selected = if local.maximum < peer.maximum {
        local.maximum
    } else {
        peer.maximum
    };
    if selected < local.minimum || selected < peer.minimum {
        Err(ProtocolError::NoCommonVersion)
    } else {
        Ok(selected)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    InvalidVersionRange,
    NoCommonVersion,
    WrongTrafficClass,
    NotNegotiated,
    MessageTooLarge { limit: usize, actual: usize },
    InvalidSequence,
    Replay,
    SequenceTooOld,
    AuthenticationFailed,
    AuthenticationLocked,
    Backpressure,
    InvalidRelease,
    ReconnectExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayWindow {
    highest: u64,
    seen: u64,
    initialized: bool,
}

impl ReplayWindow {
    const fn new() -> Self {
        Self {
            highest: 0,
            seen: 0,
            initialized: false,
        }
    }

    fn accept(&mut self, sequence: u64) -> Result<(), ProtocolError> {
        if sequence == 0 {
            return Err(ProtocolError::InvalidSequence)
        }
        if !self.initialized {
            self.initialized = true;
            self.highest = sequence;
            self.seen = 1;
            return Ok(())
        }
        if sequence > self.highest {
            let shift = sequence - self.highest;
            self.seen = if shift >= REPLAY_WINDOW_BITS as u64 {
                1
            } else {
                (self.seen << shift) | 1
            };
            self.highest = sequence;
            return Ok(())
        }
        let offset = self.highest - sequence;
        if offset >= REPLAY_WINDOW_BITS as u64 {
            return Err(ProtocolError::SequenceTooOld)
        }
        let bit = 1u64 << offset;
        if self.seen & bit != 0 {
            return Err(ProtocolError::Replay)
        }
        self.seen |= bit;
        Ok(())
    }

    const fn highest(self) -> Option<u64> {
        if self.initialized {
            Some(self.highest)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuthTracker {
    failures: u8,
    locked: bool,
}

impl AuthTracker {
    const fn new() -> Self {
        Self {
            failures: 0,
            locked: false,
        }
    }

    fn record(&mut self, success: bool, limit: u8) -> Result<(), ProtocolError> {
        if self.locked {
            return Err(ProtocolError::AuthenticationLocked)
        }
        if success {
            self.failures = 0;
            return Ok(())
        }
        self.failures = self.failures.saturating_add(1);
        if self.failures >= limit {
            self.locked = true;
            Err(ProtocolError::AuthenticationLocked)
        } else {
            Err(ProtocolError::AuthenticationFailed)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Backpressure {
    bytes: usize,
    messages: u16,
}

impl Backpressure {
    const fn new() -> Self {
        Self {
            bytes: 0,
            messages: 0,
        }
    }

    fn reserve(&mut self, bytes: usize, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        let next_bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(ProtocolError::Backpressure)?;
        let next_messages = self
            .messages
            .checked_add(1)
            .ok_or(ProtocolError::Backpressure)?;
        if next_bytes > limits.max_inflight_bytes
            || next_messages > limits.max_inflight_messages
        {
            return Err(ProtocolError::Backpressure)
        }
        self.bytes = next_bytes;
        self.messages = next_messages;
        Ok(())
    }

    fn release(&mut self, bytes: usize) -> Result<(), ProtocolError> {
        if self.messages == 0 || bytes > self.bytes {
            return Err(ProtocolError::InvalidRelease)
        }
        self.bytes -= bytes;
        self.messages -= 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReconnectState {
    attempts: u8,
    next_retry_at_us: u64,
    exhausted: bool,
}

impl ReconnectState {
    const fn new() -> Self {
        Self {
            attempts: 0,
            next_retry_at_us: 0,
            exhausted: false,
        }
    }

    fn disconnected(
        &mut self,
        now_us: u64,
        base_delay_us: u64,
    ) -> Result<u64, ProtocolError> {
        if self.attempts >= 8 {
            self.exhausted = true;
            return Err(ProtocolError::ReconnectExhausted)
        }
        let shift = self.attempts as u32;
        let multiplier = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let delay = base_delay_us.saturating_mul(multiplier);
        self.attempts += 1;
        self.next_retry_at_us = now_us.saturating_add(delay);
        Ok(self.next_retry_at_us)
    }

    const fn due(self, now_us: u64) -> bool {
        self.attempts != 0 && !self.exhausted && now_us >= self.next_retry_at_us
    }

    const fn reset(&mut self) {
        self.attempts = 0;
        self.next_retry_at_us = 0;
        self.exhausted = false;
    }
}

/// State shared by all six network-facing protocol boundaries.
pub struct ProtocolGuard {
    class: TrafficClass,
    limits: ProtocolLimits,
    local_versions: VersionRange,
    negotiated_version: Option<u16>,
    replay: ReplayWindow,
    auth: AuthTracker,
    backpressure: Backpressure,
    reconnect: ReconnectState,
}

impl ProtocolGuard {
    pub fn new(class: TrafficClass, local_versions: VersionRange) -> Result<Self, ProtocolError> {
        if local_versions.minimum == 0 || local_versions.minimum > local_versions.maximum {
            return Err(ProtocolError::InvalidVersionRange)
        }
        Ok(Self {
            class,
            limits: class.limits(),
            local_versions,
            negotiated_version: None,
            replay: ReplayWindow::new(),
            auth: AuthTracker::new(),
            backpressure: Backpressure::new(),
            reconnect: ReconnectState::new(),
        })
    }

    pub const fn class(&self) -> TrafficClass {
        self.class
    }

    pub fn require_class(&self, class: TrafficClass) -> Result<(), ProtocolError> {
        if self.class == class {
            Ok(())
        } else {
            Err(ProtocolError::WrongTrafficClass)
        }
    }

    pub const fn limits(&self) -> ProtocolLimits {
        self.limits
    }

    pub const fn negotiated_version(&self) -> Option<u16> {
        self.negotiated_version
    }

    pub fn negotiate(&mut self, peer_versions: VersionRange) -> Result<u16, ProtocolError> {
        let selected = negotiate_versions(self.local_versions, peer_versions)?;
        self.negotiated_version = Some(selected);
        Ok(selected)
    }

    pub fn validate_message(&self, bytes: usize) -> Result<(), ProtocolError> {
        if self.negotiated_version.is_none() {
            return Err(ProtocolError::NotNegotiated)
        }
        if bytes > self.limits.max_message_bytes {
            return Err(ProtocolError::MessageTooLarge {
                limit: self.limits.max_message_bytes,
                actual: bytes,
            })
        }
        Ok(())
    }

    pub fn accept_sequence(&mut self, sequence: u64) -> Result<(), ProtocolError> {
        self.replay.accept(sequence)
    }

    pub const fn highest_sequence(&self) -> Option<u64> {
        self.replay.highest()
    }

    pub fn authenticate(&mut self, success: bool) -> Result<(), ProtocolError> {
        self.auth.record(success, self.limits.max_auth_failures)
    }

    pub const fn auth_failures_locked(&self) -> bool {
        self.auth.locked
    }

    pub fn reserve_message(&mut self, bytes: usize) -> Result<(), ProtocolError> {
        self.validate_message(bytes)?;
        self.backpressure.reserve(bytes, self.limits)
    }

    pub fn release_message(&mut self, bytes: usize) -> Result<(), ProtocolError> {
        self.backpressure.release(bytes)
    }

    pub const fn inflight(&self) -> (usize, u16) {
        (self.backpressure.bytes, self.backpressure.messages)
    }

    pub fn disconnected(&mut self, now_us: u64) -> Result<u64, ProtocolError> {
        self.reconnect
            .disconnected(now_us, self.limits.reconnect_base_delay_us)
    }

    pub const fn reconnect_due(&self, now_us: u64) -> bool {
        self.reconnect.due(now_us)
    }

    pub const fn reconnect_attempts(&self) -> u8 {
        self.reconnect.attempts
    }

    pub const fn reconnected(&mut self) {
        self.reconnect.reset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TRAFFIC: [TrafficClass; 6] = [
        TrafficClass::Http,
        TrafficClass::Grpc,
        TrafficClass::Sdk,
        TrafficClass::RemoteTerminal,
        TrafficClass::Mesh,
        TrafficClass::Cluster,
    ];

    fn guard(class: TrafficClass) -> ProtocolGuard {
        ProtocolGuard::new(class, VersionRange::new(1, 2).expect("valid range")).expect("guard")
    }

    #[test]
    fn every_boundary_uses_negotiation_limits_replay_auth_flow_control_and_reconnect() {
        for class in ALL_TRAFFIC {
            let mut guard = guard(class);
            assert_eq!(
                guard.negotiate(VersionRange::new(1, 1).expect("valid range")),
                Ok(1)
            );
            assert_eq!(guard.negotiated_version(), Some(1));
            assert_eq!(guard.validate_message(guard.limits().max_message_bytes), Ok(()));
            assert_eq!(
                guard.validate_message(guard.limits().max_message_bytes + 1),
                Err(ProtocolError::MessageTooLarge {
                    limit: guard.limits().max_message_bytes,
                    actual: guard.limits().max_message_bytes + 1,
                })
            );
            assert_eq!(guard.accept_sequence(10), Ok(()));
            assert_eq!(guard.accept_sequence(10), Err(ProtocolError::Replay));
            assert_eq!(guard.accept_sequence(9), Ok(()));
            assert_eq!(guard.accept_sequence(75), Ok(()));
            assert_eq!(guard.accept_sequence(10), Err(ProtocolError::SequenceTooOld));
            assert_eq!(guard.authenticate(false), Err(ProtocolError::AuthenticationFailed));
            assert_eq!(guard.authenticate(true), Ok(()));
            assert_eq!(guard.reserve_message(1), Ok(()));
            assert_eq!(guard.inflight(), (1, 1));
            assert_eq!(guard.release_message(1), Ok(()));
            assert_eq!(guard.release_message(1), Err(ProtocolError::InvalidRelease));
            let retry_at = guard.disconnected(100);
            assert_eq!(retry_at, Ok(100 + guard.limits().reconnect_base_delay_us));
            assert!(!guard.reconnect_due(retry_at.unwrap() - 1));
            assert!(guard.reconnect_due(retry_at.unwrap()));
            guard.reconnected();
            assert_eq!(guard.reconnect_attempts(), 0);
        }
    }

    #[test]
    fn incompatible_versions_and_repeated_auth_failures_fail_closed() {
        assert_eq!(
            negotiate_versions(
                VersionRange::new(2, 3).expect("valid range"),
                VersionRange::new(1, 1).expect("valid range"),
            ),
            Err(ProtocolError::NoCommonVersion)
        );
        let mut guard = guard(TrafficClass::Http);
        assert_eq!(guard.authenticate(false), Err(ProtocolError::AuthenticationFailed));
        assert_eq!(guard.authenticate(false), Err(ProtocolError::AuthenticationFailed));
        assert_eq!(guard.authenticate(false), Err(ProtocolError::AuthenticationLocked));
        assert!(guard.auth_failures_locked());
        assert_eq!(guard.authenticate(true), Err(ProtocolError::AuthenticationLocked));
    }

    #[test]
    fn backpressure_rejects_saturation_and_reconnect_has_bounded_attempts() {
        let mut guard = guard(TrafficClass::RemoteTerminal);
        guard.negotiate(VersionRange::new(1, 1).expect("valid range")).expect("negotiates");
        let limit = guard.limits().max_inflight_bytes;
        assert_eq!(guard.reserve_message(limit), Ok(()));
        assert_eq!(guard.reserve_message(1), Err(ProtocolError::Backpressure));
        assert_eq!(guard.release_message(limit), Ok(()));
        for attempt in 0..8 {
            assert_eq!(guard.disconnected(attempt as u64), Ok(attempt as u64 + guard.limits().reconnect_base_delay_us.saturating_mul(1u64 << attempt)));
        }
        assert_eq!(guard.disconnected(9), Err(ProtocolError::ReconnectExhausted));
    }
}
