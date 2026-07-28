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
}

pub trait IntoStatus {
    fn status(self) -> Status;
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
}
