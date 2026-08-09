#![no_std]
#![forbid(unsafe_code)]

const SEVERITY_MASK: u32 = 0x7;
const CODE_MASK: u32 = 0x1fff;
const FACILITY_MASK: u32 = 0x0fff;
const FLAGS_MASK: u32 = 0xf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Severity {
    Warning = 0,
    Success = 1,
    Error = 2,
    Information = 3,
    Fatal = 4,
}

impl Severity {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Warning),
            1 => Some(Self::Success),
            2 => Some(Self::Error),
            3 => Some(Self::Information),
            4 => Some(Self::Fatal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Status(u32);

impl Status {
    pub const NORMAL: Self =
        Self::new(Severity::Success, facility::SYSTEM, 1, 0).expect("valid status");
    pub const PENDING: Self =
        Self::new(Severity::Information, facility::SYSTEM, 2, 0).expect("valid status");
    pub const INVALID_ARGUMENT: Self =
        Self::new(Severity::Error, facility::SYSTEM, 3, 0).expect("valid status");
    pub const NOT_FOUND: Self =
        Self::new(Severity::Error, facility::SYSTEM, 4, 0).expect("valid status");
    pub const ACCESS_DENIED: Self =
        Self::new(Severity::Error, facility::SECURITY, 1, 0).expect("valid status");
    pub const NO_SPACE: Self =
        Self::new(Severity::Error, facility::SYSTEM, 5, 0).expect("valid status");
    pub const CORRUPT: Self =
        Self::new(Severity::Fatal, facility::SYSTEM, 6, 0).expect("valid status");
    pub const BUSY: Self =
        Self::new(Severity::Warning, facility::SYSTEM, 7, 0).expect("valid status");
    pub const CANCELLED: Self =
        Self::new(Severity::Warning, facility::SYSTEM, 8, 0).expect("valid status");
    pub const INTERNAL: Self =
        Self::new(Severity::Error, facility::SYSTEM, 9, 0).expect("valid status");
    pub const METHOD_NOT_ALLOWED: Self =
        Self::new(Severity::Error, facility::SYSTEM, 10, 0).expect("valid status");
    pub const REQUEST_TOO_LARGE: Self =
        Self::new(Severity::Error, facility::SYSTEM, 11, 0).expect("valid status");
    pub const ALREADY_EXISTS: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 2, 0).expect("valid status");
    pub const CONFLICT: Self =
        Self::new(Severity::Warning, facility::FILESYSTEM, 3, 0).expect("valid status");
    pub const DIRECTORY_NOT_EMPTY: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 5, 0).expect("valid status");
    pub const INVALID_PATH: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 6, 0).expect("valid status");
    pub const NOT_DIRECTORY: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 7, 0).expect("valid status");
    pub const READ_ONLY: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 8, 0).expect("valid status");
    pub const PARTIAL_MATCH: Self =
        Self::new(Severity::Warning, facility::FILESYSTEM, 9, 0).expect("valid status");
    pub const INVALID_PATTERN: Self =
        Self::new(Severity::Error, facility::FILESYSTEM, 10, 0).expect("valid status");
    pub const QUORUM_LOST: Self =
        Self::new(Severity::Warning, facility::FABRIC, 1, 0).expect("valid status");
    pub const PARTITIONED: Self =
        Self::new(Severity::Warning, facility::FABRIC, 2, 0).expect("valid status");
    pub const CLOCK_SKEW: Self =
        Self::new(Severity::Error, facility::FABRIC, 3, 0).expect("valid status");
    pub const PROTOCOL_MISMATCH: Self =
        Self::new(Severity::Error, facility::FABRIC, 4, 0).expect("valid status");
    pub const CLUSTER_DEGRADED: Self =
        Self::new(Severity::Warning, facility::FABRIC, 11, 0).expect("valid status");
    pub const STALE_STATE: Self =
        Self::new(Severity::Warning, facility::FABRIC, 6, 0).expect("valid status");
    pub const NODE_UNSAFE: Self =
        Self::new(Severity::Error, facility::FABRIC, 7, 0).expect("valid status");
    pub const CONFIRMATION_REQUIRED: Self =
        Self::new(Severity::Error, facility::FABRIC, 8, 0).expect("valid status");
    pub const RECONCILIATION_REQUIRED: Self =
        Self::new(Severity::Warning, facility::FABRIC, 9, 0).expect("valid status");
    pub const ROLLBACK_UNAVAILABLE: Self =
        Self::new(Severity::Error, facility::FABRIC, 10, 0).expect("valid status");
    pub const RECOVERY_STATE_INVALID: Self =
        Self::new(Severity::Error, facility::FABRIC, 12, 0).expect("valid status");

    pub const fn new(severity: Severity, facility: u16, code: u16, flags: u8) -> Option<Self> {
        if facility as u32 > FACILITY_MASK || code as u32 > CODE_MASK || flags as u32 > FLAGS_MASK {
            return None;
        }
        Some(Self(
            severity as u32
                | ((code as u32) << 3)
                | ((facility as u32) << 16)
                | ((flags as u32) << 28),
        ))
    }

    pub const fn from_raw(raw: u32) -> Option<Self> {
        if Severity::from_raw((raw & SEVERITY_MASK) as u8).is_none() {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn severity(self) -> Severity {
        match Severity::from_raw((self.0 & SEVERITY_MASK) as u8) {
            Some(severity) => severity,
            None => unreachable!(),
        }
    }

    pub const fn facility(self) -> u16 {
        ((self.0 >> 16) & FACILITY_MASK) as u16
    }

    pub const fn code(self) -> u16 {
        ((self.0 >> 3) & CODE_MASK) as u16
    }

    pub const fn flags(self) -> u8 {
        ((self.0 >> 28) & FLAGS_MASK) as u8
    }

    /// OpenVMS-compatible low-bit convention: odd values indicate success.
    pub const fn is_success(self) -> bool {
        self.0 & 1 == 1
    }

    pub const fn message(self) -> &'static str {
        match (self.facility(), self.code()) {
            (facility::SYSTEM, 1) => "normal",
            (facility::SYSTEM, 2) => "pending",
            (facility::SYSTEM, 3) => "invalid argument",
            (facility::SYSTEM, 4) => "not found",
            (facility::SYSTEM, 5) => "no space",
            (facility::SYSTEM, 6) => "corrupt",
            (facility::SYSTEM, 7) => "busy",
            (facility::SYSTEM, 8) => "cancelled",
            (facility::SYSTEM, 9) => "internal error",
            (facility::SYSTEM, 10) => "method not allowed",
            (facility::SYSTEM, 11) => "request too large",
            (facility::SECURITY, 1) => "access denied",
            (facility::FILESYSTEM, 2) => "already exists",
            (facility::FILESYSTEM, 3) => "conflict",
            (facility::FILESYSTEM, 5) => "directory not empty",
            (facility::FILESYSTEM, 6) => "invalid path",
            (facility::FILESYSTEM, 7) => "not a directory",
            (facility::FILESYSTEM, 8) => "read-only mount",
            (facility::FILESYSTEM, 9) => "partial wildcard match",
            (facility::FILESYSTEM, 10) => "malformed wildcard pattern",
            (facility::FABRIC, 1) => "quorum lost",
            (facility::FABRIC, 2) => "cluster partitioned",
            (facility::FABRIC, 3) => "clock skew",
            (facility::FABRIC, 4) => "protocol mismatch",
            (facility::FABRIC, 11) => "cluster degraded",
            (facility::FABRIC, 6) => "stale state",
            (facility::FABRIC, 7) => "node unsafe",
            (facility::FABRIC, 8) => "confirmation required",
            (facility::FABRIC, 9) => "reconciliation required",
            (facility::FABRIC, 10) => "rollback unavailable",
            (facility::FABRIC, 12) => "recovery state invalid",
            _ => "unknown status",
        }
    }

    pub const fn retry_hint(self) -> RetryHint {
        match self {
            Self::BUSY | Self::NO_SPACE | Self::INTERNAL => RetryHint::AfterUs(1_000_000),
            _ => RetryHint::Never,
        }
    }

    pub const fn public_error(self, operation: u16, audit: AuditContext) -> PublicError {
        PublicError::new(self, operation, self.retry_hint(), audit)
    }
}

pub trait IntoStatus {
    fn status(self) -> Status;
}

/// Converts any status-bearing error into the safe boundary contract.
pub trait IntoPublicError: IntoStatus + Sized {
    fn public_error(self, operation: u16, audit: AuditContext) -> PublicError {
        self.status().public_error(operation, audit)
    }
}

impl<T: IntoStatus> IntoPublicError for T {}

/// The retry advice exposed at a service boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryHint {
    Never,
    Immediate,
    AfterUs(u64),
}

impl RetryHint {
    pub const fn is_retryable(self) -> bool {
        !matches!(self, Self::Never)
    }
}

/// Safe correlation data that lets an operator find the matching audit event.
///
/// This deliberately contains identifiers only. It must not contain request
/// bodies, paths, credentials, capabilities, or backend error strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct AuditContext {
    pub correlation: u128,
    pub node: u32,
}

impl AuditContext {
    pub const NONE: Self = Self {
        correlation: 0,
        node: 0,
    };

    pub const fn new(correlation: u128, node: u32) -> Self {
        Self { correlation, node }
    }
}

/// Stable, externally safe error metadata shared by service boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicError {
    pub code: Status,
    pub operation: u16,
    pub retry: RetryHint,
    pub audit: AuditContext,
}

impl PublicError {
    pub const fn new(
        code: Status,
        operation: u16,
        retry: RetryHint,
        audit: AuditContext,
    ) -> Self {
        Self {
            code,
            operation,
            retry,
            audit,
        }
    }
}

/// Stable operation identifiers used by the externally visible HTTP/RPC
/// boundaries. These are identifiers, not user-controlled strings.
pub mod operation {
    pub const HTTP_PARSE: u16 = 1;
    pub const HTTP_ROUTE: u16 = 2;
    pub const HTTP_RPC: u16 = 3;
    pub const HTTP_SERVER: u16 = 4;
    pub const GRPC: u16 = 5;
    pub const FRONTEND_RPC: u16 = 6;
}

pub mod facility {
    pub const SYSTEM: u16 = 1;
    pub const KERNEL: u16 = 2;
    pub const FILESYSTEM: u16 = 3;
    pub const DRIVER: u16 = 4;
    pub const COMMAND: u16 = 5;
    pub const LOGICAL_NAME: u16 = 6;
    pub const SECURITY: u16 = 7;
    pub const DLM: u16 = 8;
    pub const RMS: u16 = 9;
    pub const FABRIC: u16 = 10;
    pub const LLM: u16 = 11;
    pub const SHELL: u16 = 12;
    pub const NETWORK: u16 = 13;
    pub const COMPUTE: u16 = 14;
    pub const SCRIPT: u16 = 15;
}

#[cfg(test)]
mod tests;
