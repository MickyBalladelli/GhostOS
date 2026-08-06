//! Frozen design contract for the native compiler service.

use crate::{BuildRequest, BuildResult, Diagnostic, JobId, JobState, Status, Target, Text};

pub const COMPILER_PROTOCOL_VERSION: u16 = 1;
pub const MAX_LOG_MESSAGE_BYTES: usize = crate::MAX_DIAGNOSTIC_BYTES;

/// The first self-hosting target. Aarch64 stays available as a build target,
/// but becomes self-hosting only after this path is complete.
pub const PRIMARY_SELF_HOST_TARGET: Target = Target::X86_64;
pub const NEXT_SELF_HOST_TARGET: Target = Target::Aarch64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TargetSupport {
    PrimarySelfHost = 1,
    PlannedSelfHost = 2,
}

impl TargetSupport {
    pub const fn for_target(target: Target) -> Self {
        match target {
            PRIMARY_SELF_HOST_TARGET => Self::PrimarySelfHost,
            NEXT_SELF_HOST_TARGET => Self::PlannedSelfHost,
        }
    }

    pub const fn is_self_hosting(self) -> bool {
        matches!(self, Self::PrimarySelfHost)
    }
}

/// Rust components included in the initial supported surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RustSurface(u16);

impl RustSurface {
    pub const CORE: Self = Self(1 << 0);
    pub const ALLOC: Self = Self(1 << 1);
    pub const STD: Self = Self(1 << 2);
    pub const CARGO: Self = Self(1 << 3);
    pub const BUILD_SCRIPTS: Self = Self(1 << 4);
    pub const PROC_MACROS: Self = Self(1 << 5);
    pub const TESTS: Self = Self(1 << 6);
    pub const RUSTDOC: Self = Self(1 << 7);

    pub const INITIAL: Self = Self(
        Self::CORE.0
            | Self::ALLOC.0
            | Self::STD.0
            | Self::CARGO.0
            | Self::BUILD_SCRIPTS.0
            | Self::PROC_MACROS.0
            | Self::TESTS.0
            | Self::RUSTDOC.0,
    );

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, component: Self) -> bool {
        self.0 & component.0 == component.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompilerStack {
    UpstreamRustcLlvm = 1,
}

impl CompilerStack {
    pub const fn name(self) -> &'static str {
        match self {
            Self::UpstreamRustcLlvm => "upstream rustc plus LLVM",
        }
    }
}

pub const SYNFS_TOOLCHAINS: &str = "/system/toolchains";
pub const SYNFS_BOOTSTRAP_TOOLCHAIN: &str = "/system/toolchains/stage-0";
pub const SYNFS_RUNTIME_TOOLCHAIN: &str = "/system/toolchains/stage-1";
pub const SYNFS_SELF_HOST_TOOLCHAIN: &str = "/system/toolchains/stage-2";
pub const SYNFS_REGISTRIES: &str = "/system/registries";
pub const SYNFS_SOURCES: &str = "/system/sources";
pub const SYNFS_BUILD_STATE: &str = "/system/builds";
pub const SYNFS_TEMP: &str = "/system/tmp";
pub const SYNFS_OUTPUT_BUNDLES: &str = "/system/bundles";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StorageArea {
    Toolchains = 1,
    Registries = 2,
    Sources = 3,
    BuildState = 4,
    Temporary = 5,
    OutputBundles = 6,
}

impl StorageArea {
    pub const fn path(self) -> &'static str {
        match self {
            Self::Toolchains => SYNFS_TOOLCHAINS,
            Self::Registries => SYNFS_REGISTRIES,
            Self::Sources => SYNFS_SOURCES,
            Self::BuildState => SYNFS_BUILD_STATE,
            Self::Temporary => SYNFS_TEMP,
            Self::OutputBundles => SYNFS_OUTPUT_BUNDLES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum CompilerOperation {
    Submit = 1,
    Start = 2,
    Poll = 3,
    Cancel = 4,
    Release = 5,
    ReadLog = 6,
}

impl CompilerOperation {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Submit),
            2 => Some(Self::Start),
            3 => Some(Self::Poll),
            4 => Some(Self::Cancel),
            5 => Some(Self::Release),
            6 => Some(Self::ReadLog),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompilerLogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompilerEventKind {
    Queued = 1,
    Started = 2,
    Progress = 3,
    Diagnostic = 4,
    Completed = 5,
    Failed = 6,
    Cancelled = 7,
}

impl CompilerEventKind {
    pub const fn status(self) -> Status {
        match self {
            Self::Queued | Self::Started | Self::Progress => Status::PENDING,
            Self::Diagnostic => Status::NORMAL,
            Self::Completed => Status::NORMAL,
            Self::Failed => Status::BUSY,
            Self::Cancelled => Status::CANCELLED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompilerLogRecord {
    pub job: u64,
    pub sequence: u32,
    pub level: CompilerLogLevel,
    pub event: CompilerEventKind,
    pub message: Text<MAX_LOG_MESSAGE_BYTES>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompilerProtocolError {
    VersionMismatch = 1,
    InvalidOperation = 2,
    MissingBuild = 3,
    UnexpectedBuild = 4,
    MissingJob = 5,
    InvalidBuild = 6,
}

impl CompilerProtocolError {
    pub const fn status(self) -> Status {
        match self {
            Self::VersionMismatch => Status::PROTOCOL_MISMATCH,
            Self::InvalidOperation
            | Self::MissingBuild
            | Self::UnexpectedBuild
            | Self::MissingJob
            | Self::InvalidBuild => Status::INVALID_ARGUMENT,
        }
    }
}

/// Fixed-size request frame. The optional fields occupy fixed wire space;
/// they are presence flags, not heap-backed values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompilerIpcRequest {
    pub version: u16,
    pub operation: CompilerOperation,
    pub job: u64,
    pub build: Option<BuildRequest>,
    pub now_us: u64,
    pub log_sequence: u32,
}

impl CompilerIpcRequest {
    pub const fn new(operation: CompilerOperation) -> Self {
        Self {
            version: COMPILER_PROTOCOL_VERSION,
            operation,
            job: 0,
            build: None,
            now_us: 0,
            log_sequence: 0,
        }
    }

    pub fn validate(&self) -> Result<(), CompilerProtocolError> {
        if self.version != COMPILER_PROTOCOL_VERSION {
            return Err(CompilerProtocolError::VersionMismatch);
        }

        match self.operation {
            CompilerOperation::Submit => {
                if self.job != 0 {
                    return Err(CompilerProtocolError::UnexpectedBuild);
                }
                let build = self.build.ok_or(CompilerProtocolError::MissingBuild)?;
                build
                    .validate()
                    .map_err(|_| CompilerProtocolError::InvalidBuild)
            }
            CompilerOperation::Start
            | CompilerOperation::Poll
            | CompilerOperation::Cancel
            | CompilerOperation::Release
            | CompilerOperation::ReadLog => {
                if self.job == 0 {
                    return Err(CompilerProtocolError::MissingJob);
                }
                if self.build.is_some() {
                    return Err(CompilerProtocolError::UnexpectedBuild);
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompilerIpcResponse {
    pub version: u16,
    pub status: Status,
    pub job: Option<JobId>,
    pub state: Option<JobState>,
    pub event: Option<CompilerEventKind>,
    pub sequence: u32,
    pub diagnostic: Option<Diagnostic>,
    pub result: Option<BuildResult>,
    pub log: Option<CompilerLogRecord>,
}

impl CompilerIpcResponse {
    pub const fn empty(status: Status) -> Self {
        Self {
            version: COMPILER_PROTOCOL_VERSION,
            status,
            job: None,
            state: None,
            event: None,
            sequence: 0,
            diagnostic: None,
            result: None,
            log: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CancellationDisposition {
    CancelQueued = 1,
    RequestCooperativeStop = 2,
    AlreadyTerminal = 3,
}

impl CancellationDisposition {
    pub const fn for_state(state: JobState) -> Self {
        match state {
            JobState::Queued => Self::CancelQueued,
            JobState::Running => Self::RequestCooperativeStop,
            JobState::Completed | JobState::Failed | JobState::Cancelled => Self::AlreadyTerminal,
        }
    }
}

/// Cancellation is cooperative while compiler processes run. The supervisor
/// fences the process after the grace period and publishes no partial output.
pub const CANCELLATION_GRACE_US: u64 = 5_000_000;
