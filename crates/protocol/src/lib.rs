#![no_std]

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
#[repr(C)]
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
#[repr(C)]
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

#[repr(C)]
struct GuardState {
    class: TrafficClass,
    limits: ProtocolLimits,
    local_versions: VersionRange,
    negotiated_version: u16,
    highest_sequence: u64,
    seen: u64,
    replay_initialized: bool,
    auth_failures: u8,
    auth_locked: bool,
    inflight_bytes: usize,
    inflight_messages: u16,
    reconnect_attempts: u8,
    next_retry_at_us: u64,
    reconnect_exhausted: bool,
}

#[repr(C)]
struct NativeError {
    kind: u32,
    limit: usize,
    actual: usize,
}

impl NativeError {
    fn result(self) -> Result<(), ProtocolError> {
        Err(match self.kind {
            0 => return Ok(()),
            1 => ProtocolError::InvalidVersionRange,
            2 => ProtocolError::NoCommonVersion,
            3 => ProtocolError::WrongTrafficClass,
            4 => ProtocolError::NotNegotiated,
            5 => ProtocolError::MessageTooLarge { limit: self.limit, actual: self.actual },
            6 => ProtocolError::InvalidSequence,
            7 => ProtocolError::Replay,
            8 => ProtocolError::SequenceTooOld,
            9 => ProtocolError::AuthenticationFailed,
            10 => ProtocolError::AuthenticationLocked,
            11 => ProtocolError::Backpressure,
            12 => ProtocolError::InvalidRelease,
            13 => ProtocolError::ReconnectExhausted,
            _ => panic!("invalid C protocol result"),
        })
    }
}

/// State shared by all six network-facing protocol boundaries.
pub struct ProtocolGuard {
    state: GuardState,
}

impl ProtocolGuard {
    pub fn new(class: TrafficClass, local_versions: VersionRange) -> Result<Self, ProtocolError> {
        let mut state = GuardState {
            class,
            limits: class.limits(),
            local_versions,
            negotiated_version: 0,
            highest_sequence: 0,
            seen: 0,
            replay_initialized: false,
            auth_failures: 0,
            auth_locked: false,
            inflight_bytes: 0,
            inflight_messages: 0,
            reconnect_attempts: 0,
            next_retry_at_us: 0,
            reconnect_exhausted: false,
        };
        // C initializes checked caller-owned state; no pointers are retained.
        unsafe { ghostos_protocol_guard_new(class as u8, local_versions, &mut state) }.result()?;
        Ok(Self { state })
    }

    pub const fn class(&self) -> TrafficClass {
        self.state.class
    }

    pub fn require_class(&self, class: TrafficClass) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_require_class(&self.state, class as u8) }.result()
    }

    pub const fn limits(&self) -> ProtocolLimits {
        self.state.limits
    }

    pub const fn negotiated_version(&self) -> Option<u16> {
        if self.state.negotiated_version == 0 { None } else { Some(self.state.negotiated_version) }
    }

    pub fn negotiate(&mut self, peer_versions: VersionRange) -> Result<u16, ProtocolError> {
        let mut selected = 0;
        unsafe { ghostos_protocol_negotiate(&mut self.state, peer_versions, &mut selected) }.result()?;
        Ok(selected)
    }

    pub fn validate_message(&self, bytes: usize) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_validate_message(&self.state, bytes) }.result()
    }

    pub fn accept_sequence(&mut self, sequence: u64) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_accept_sequence(&mut self.state, sequence) }.result()
    }

    pub const fn highest_sequence(&self) -> Option<u64> {
        if self.state.replay_initialized { Some(self.state.highest_sequence) } else { None }
    }

    pub fn authenticate(&mut self, success: bool) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_authenticate(&mut self.state, success) }.result()
    }

    pub const fn auth_failures_locked(&self) -> bool {
        self.state.auth_locked
    }

    pub fn reserve_message(&mut self, bytes: usize) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_reserve_message(&mut self.state, bytes) }.result()
    }

    pub fn release_message(&mut self, bytes: usize) -> Result<(), ProtocolError> {
        unsafe { ghostos_protocol_release_message(&mut self.state, bytes) }.result()
    }

    pub const fn inflight(&self) -> (usize, u16) {
        (self.state.inflight_bytes, self.state.inflight_messages)
    }

    pub fn disconnected(&mut self, now_us: u64) -> Result<u64, ProtocolError> {
        let mut retry_at = 0;
        unsafe { ghostos_protocol_disconnected(&mut self.state, now_us, &mut retry_at) }.result()?;
        Ok(retry_at)
    }

    pub const fn reconnect_due(&self, now_us: u64) -> bool {
        self.state.reconnect_attempts != 0 && !self.state.reconnect_exhausted &&
            now_us >= self.state.next_retry_at_us
    }

    pub const fn reconnect_attempts(&self) -> u8 {
        self.state.reconnect_attempts
    }

    pub const fn reconnected(&mut self) {
        self.state.reconnect_attempts = 0;
        self.state.next_retry_at_us = 0;
        self.state.reconnect_exhausted = false;
    }
}

const _: () = {
    assert!(core::mem::size_of::<ProtocolLimits>() == 32);
    assert!(core::mem::offset_of!(ProtocolLimits, max_inflight_bytes) == 8);
    assert!(core::mem::offset_of!(ProtocolLimits, max_inflight_messages) == 16);
    assert!(core::mem::offset_of!(ProtocolLimits, max_auth_failures) == 18);
    assert!(core::mem::offset_of!(ProtocolLimits, reconnect_base_delay_us) == 24);
    assert!(core::mem::size_of::<VersionRange>() == 4);
    assert!(core::mem::offset_of!(VersionRange, maximum) == 2);
    assert!(core::mem::size_of::<NativeError>() == 24);
    assert!(core::mem::offset_of!(NativeError, limit) == 8);
    assert!(core::mem::offset_of!(NativeError, actual) == 16);
    assert!(core::mem::size_of::<GuardState>() == 104);
    assert!(core::mem::offset_of!(GuardState, limits) == 8);
    assert!(core::mem::offset_of!(GuardState, local_versions) == 40);
    assert!(core::mem::offset_of!(GuardState, negotiated_version) == 44);
    assert!(core::mem::offset_of!(GuardState, highest_sequence) == 48);
    assert!(core::mem::offset_of!(GuardState, seen) == 56);
    assert!(core::mem::offset_of!(GuardState, replay_initialized) == 64);
    assert!(core::mem::offset_of!(GuardState, auth_failures) == 65);
    assert!(core::mem::offset_of!(GuardState, auth_locked) == 66);
    assert!(core::mem::offset_of!(GuardState, inflight_bytes) == 72);
    assert!(core::mem::offset_of!(GuardState, inflight_messages) == 80);
    assert!(core::mem::offset_of!(GuardState, reconnect_attempts) == 82);
    assert!(core::mem::offset_of!(GuardState, next_retry_at_us) == 88);
    assert!(core::mem::offset_of!(GuardState, reconnect_exhausted) == 96);
};

unsafe extern "C" {
    fn ghostos_protocol_guard_new(class: u8, local: VersionRange, out: *mut GuardState) -> NativeError;
    fn ghostos_protocol_require_class(guard: *const GuardState, class: u8) -> NativeError;
    fn ghostos_protocol_negotiate(guard: *mut GuardState, peer: VersionRange, selected: *mut u16) -> NativeError;
    fn ghostos_protocol_validate_message(guard: *const GuardState, bytes: usize) -> NativeError;
    fn ghostos_protocol_accept_sequence(guard: *mut GuardState, sequence: u64) -> NativeError;
    fn ghostos_protocol_authenticate(guard: *mut GuardState, success: bool) -> NativeError;
    fn ghostos_protocol_reserve_message(guard: *mut GuardState, bytes: usize) -> NativeError;
    fn ghostos_protocol_release_message(guard: *mut GuardState, bytes: usize) -> NativeError;
    fn ghostos_protocol_disconnected(guard: *mut GuardState, now_us: u64, retry_at: *mut u64) -> NativeError;
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
