#![no_std]
#![forbid(unsafe_code)]

mod boot;
mod design;
mod self_host;
mod toolchain;

pub use boot::{
    COMPILER_CAPABILITY_PROFILE, COMPILER_SERVICE_ID, COMPILER_SERVICE_NAME, CompilerBootError,
    CompilerBootState, CompilerServiceBoot, CompilerServiceHealthCheck, NativeCompilerBootConfig,
};
pub use design::{
    CANCELLATION_GRACE_US, COMPILER_PROTOCOL_VERSION, CancellationDisposition, CompilerEventKind,
    CompilerIpcRequest, CompilerIpcResponse, CompilerLogLevel, CompilerLogRecord,
    CompilerOperation, CompilerProtocolError, CompilerStack, MAX_LOG_MESSAGE_BYTES,
    NEXT_SELF_HOST_TARGET, PRIMARY_SELF_HOST_TARGET, RustSurface, SYNFS_BOOTSTRAP_TOOLCHAIN,
    SYNFS_BUILD_STATE, SYNFS_OUTPUT_BUNDLES, SYNFS_REGISTRIES, SYNFS_RUNTIME_TOOLCHAIN,
    SYNFS_SELF_HOST_TOOLCHAIN, SYNFS_SOURCES, SYNFS_TEMP, SYNFS_TOOLCHAINS, StorageArea,
    TargetSupport,
};
pub use self_host::{
    SelfHostError, SelfHostRequest, SelfHostResult, SelfHostSession, SelfHostStage,
    ToolchainStage, ToolchainStageError, ToolchainStageResult,
};
pub use toolchain::{
    ToolExit, ToolKind, ToolSpawnRequest, ToolchainComponent, ToolchainError, ToolchainExecutor,
    ArtifactError, ArtifactSandbox, DynamicArtifact, DynamicArtifactKind, ToolchainAsset,
    ToolchainAssetKind, ToolchainExecutionError, ToolchainManifest,
    ToolchainPlan, ToolchainPolicy, ToolchainReceipt, ToolchainRequest, ToolchainRuntime,
    ToolchainStep, MAX_DYNAMIC_ARTIFACTS, MAX_TOOLCHAIN_ASSETS, MAX_TOOLCHAIN_COMPONENTS,
    MAX_TOOLCHAIN_STEPS,
};

use synos_status::{IntoStatus, Status};
use synos_synfs::{Error as SynFsError, FileType, SynFs};
use synos_system_model::ContentId;

pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_BINARY_BYTES: usize = 64;
pub const MAX_FEATURES: usize = 16;
pub const MAX_JOBS: usize = 16;
pub const MAX_CACHE_ENTRIES: usize = 32;
pub const MAX_DIAGNOSTIC_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Capacity,
    InvalidRequest,
    JobNotFound,
    InvalidTransition,
    NetworkDenied,
    ResourceLimit,
    DeadlineExpired,
    SourceFilesystem(SynFsError),
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::ResourceLimit => Status::NO_SPACE,
            Self::JobNotFound => Status::NOT_FOUND,
            Self::NetworkDenied => Status::ACCESS_DENIED,
            Self::InvalidRequest | Self::InvalidTransition | Self::DeadlineExpired => {
                Status::INVALID_ARGUMENT
            }
            Self::SourceFilesystem(error) => error.status(),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Text<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    length: u16,
}

impl<const CAPACITY: usize> Text<CAPACITY> {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; CAPACITY],
            length: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, Error> {
        let mut text = Self::empty();
        text.set(value)?;
        Ok(text)
    }

    pub fn set(&mut self, value: &str) -> Result<(), Error> {
        if value.is_empty()
            || value.len() > CAPACITY
            || CAPACITY > u16::MAX as usize
            || value.as_bytes().contains(&0)
        {
            return Err(Error::InvalidRequest);
        }
        self.bytes[..value.len()].copy_from_slice(value.as_bytes());
        self.length = value.len() as u16;
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        // Text only accepts UTF-8, so this conversion is an invariant of the type.
        core::str::from_utf8(&self.bytes[..self.length as usize]).unwrap()
    }
}

impl<const CAPACITY: usize> core::fmt::Debug for Text<CAPACITY> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("Text").field(&self.as_str()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Target {
    X86_64,
    Aarch64,
}

impl Target {
    pub const fn triple(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-synos",
            Self::Aarch64 => "aarch64-unknown-synos",
        }
    }

    pub const fn is_primary_self_host(self) -> bool {
        matches!(self, Self::X86_64)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Profile {
    Debug,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkPolicy {
    Denied,
    Allowed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub memory_bytes: u64,
    pub cpu_time_us: u64,
    pub deadline_us: u64,
}

impl ResourceLimits {
    pub const DEFAULT: Self = Self {
        memory_bytes: 256 * 1024 * 1024,
        cpu_time_us: 10 * 60 * 1_000_000,
        deadline_us: 10 * 60 * 1_000_000,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    pub source_root: Text<MAX_PATH_BYTES>,
    pub manifest: Text<MAX_PATH_BYTES>,
    pub binary: Text<MAX_BINARY_BYTES>,
    pub target: Target,
    pub profile: Profile,
    pub locked: bool,
    pub network: NetworkPolicy,
    pub limits: ResourceLimits,
    pub features: [Option<Text<MAX_BINARY_BYTES>>; MAX_FEATURES],
}

impl BuildRequest {
    pub fn validate(&self) -> Result<(), Error> {
        if self.limits.memory_bytes == 0
            || self.limits.cpu_time_us == 0
            || self.limits.deadline_us == 0
            || self.source_root.as_str().is_empty()
            || self.manifest.as_str().is_empty()
            || self.binary.as_str().is_empty()
        {
            return Err(Error::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildPolicy {
    pub network: NetworkPolicy,
    pub max_limits: ResourceLimits,
}

impl BuildPolicy {
    pub const OFFLINE: Self = Self {
        network: NetworkPolicy::Denied,
        max_limits: ResourceLimits::DEFAULT,
    };

    fn authorize(self, request: &BuildRequest) -> Result<(), Error> {
        request.validate()?;
        if request.network == NetworkPolicy::Allowed && self.network == NetworkPolicy::Denied {
            return Err(Error::NetworkDenied);
        }
        if self.network == NetworkPolicy::Denied && !request.locked {
            return Err(Error::InvalidRequest);
        }
        if request.limits.memory_bytes > self.max_limits.memory_bytes
            || request.limits.cpu_time_us > self.max_limits.cpu_time_us
            || request.limits.deadline_us > self.max_limits.deadline_us
        {
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct JobId(u64);

impl JobId {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: u16,
    pub message: Text<MAX_DIAGNOSTIC_BYTES>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildResult {
    pub package: ContentId,
    pub payload: ContentId,
    pub target: Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobStatus {
    pub id: JobId,
    pub state: JobState,
    pub request: BuildRequest,
    pub result: Option<BuildResult>,
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Clone, Copy)]
struct Job {
    status: JobStatus,
    started_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheEntry {
    pub source: ContentId,
    pub lockfile: ContentId,
    pub target: Target,
    pub profile: Profile,
    pub result: BuildResult,
}

pub struct CompilerService<const JOB_CAPACITY: usize = MAX_JOBS, const CACHE_CAPACITY: usize = MAX_CACHE_ENTRIES> {
    policy: BuildPolicy,
    next_id: u64,
    jobs: [Option<Job>; JOB_CAPACITY],
    cache: [Option<CacheEntry>; CACHE_CAPACITY],
    artifacts: ArtifactSandbox,
}

impl<const JOB_CAPACITY: usize, const CACHE_CAPACITY: usize>
    CompilerService<JOB_CAPACITY, CACHE_CAPACITY>
{
    pub const fn new(policy: BuildPolicy) -> Self {
        Self {
            policy,
            next_id: 1,
            jobs: [None; JOB_CAPACITY],
            cache: [None; CACHE_CAPACITY],
            artifacts: ArtifactSandbox::new(),
        }
    }

    pub fn stage_artifact<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &synos_pkg::PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        self.artifacts.stage(packages, artifact)
    }

    pub fn release_artifact(&mut self, payload: ContentId) -> Result<(), ArtifactError> {
        self.artifacts.release(payload)
    }

    pub fn release_build_artifacts(&mut self) {
        self.artifacts.release_all();
    }

    pub fn active_artifacts(&self) -> impl Iterator<Item = DynamicArtifact> + '_ {
        self.artifacts.active()
    }

    pub fn submit(&mut self, request: BuildRequest) -> Result<JobId, Error> {
        self.policy.authorize(&request)?;
        let slot = self
            .jobs
            .iter_mut()
            .find(|job| job.is_none())
            .ok_or(Error::Capacity)?;
        let id = JobId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Capacity)?;
        *slot = Some(Job {
            status: JobStatus {
                id,
                state: JobState::Queued,
                request,
                result: None,
                diagnostic: None,
            },
            started_at_us: 0,
        });
        Ok(id)
    }

    /// Validate a source tree through SynFS before putting the build on the
    /// bounded job queue. The manifest must live below the source root.
    pub fn submit_from_synfs<const BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<BLOCKS>,
        request: BuildRequest,
    ) -> Result<JobId, Error> {
        let root = filesystem
            .lookup(request.source_root.as_str())
            .map_err(Error::SourceFilesystem)?;
        if root.file_type != FileType::Directory {
            return Err(Error::InvalidRequest);
        }

        let manifest = filesystem
            .lookup(request.manifest.as_str())
            .map_err(Error::SourceFilesystem)?;
        if manifest.file_type != FileType::Regular
            || !is_source_member(request.source_root.as_str(), request.manifest.as_str())
        {
            return Err(Error::InvalidRequest);
        }

        self.submit(request)
    }

    pub fn start(&mut self, id: JobId) -> Result<(), Error> {
        self.start_at(id, 0)
    }

    pub fn start_at(&mut self, id: JobId, now_us: u64) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if job.status.state != JobState::Queued {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Running;
        job.started_at_us = now_us;
        Ok(())
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        for job in self.jobs.iter_mut().flatten() {
            if job.status.state == JobState::Running
                && job.started_at_us != 0
                && now_us.saturating_sub(job.started_at_us) >= job.status.request.limits.deadline_us
            {
                job.status.state = JobState::Failed;
                job.status.diagnostic = Some(Diagnostic {
                    code: 408,
                    message: Text::new("build deadline expired")
                        .expect("static diagnostic fits fixed buffer"),
                });
                expired += 1;
            }
        }
        expired
    }

    pub fn complete(&mut self, id: JobId, result: BuildResult) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if job.status.state != JobState::Running {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Completed;
        job.status.result = Some(result);
        Ok(())
    }

    pub fn fail(&mut self, id: JobId, diagnostic: Diagnostic) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if !matches!(job.status.state, JobState::Queued | JobState::Running) {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Failed;
        job.status.diagnostic = Some(diagnostic);
        Ok(())
    }

    pub fn cancel(&mut self, id: JobId) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if !matches!(job.status.state, JobState::Queued | JobState::Running) {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Cancelled;
        Ok(())
    }

    pub fn status(&self, id: JobId) -> Result<JobStatus, Error> {
        self.jobs
            .iter()
            .flatten()
            .find(|job| job.status.id == id)
            .map(|job| job.status)
            .ok_or(Error::JobNotFound)
    }

    pub fn release(&mut self, id: JobId) -> Result<(), Error> {
        let slot = self
            .jobs
            .iter_mut()
            .find(|job| job.is_some_and(|job| job.status.id == id))
            .ok_or(Error::JobNotFound)?;
        *slot = None;
        Ok(())
    }

    pub fn cache_lookup(
        &self,
        source: ContentId,
        lockfile: ContentId,
        target: Target,
        profile: Profile,
    ) -> Option<BuildResult> {
        self.cache
            .iter()
            .flatten()
            .find(|entry| {
                entry.source == source
                    && entry.lockfile == lockfile
                    && entry.target == target
                    && entry.profile == profile
            })
            .map(|entry| entry.result)
    }

    pub fn cache_insert(&mut self, entry: CacheEntry) -> Result<(), Error> {
        if let Some(existing) = self.cache.iter_mut().flatten().find(|current| {
            current.source == entry.source
                && current.lockfile == entry.lockfile
                && current.target == entry.target
                && current.profile == entry.profile
        }) {
            *existing = entry;
            return Ok(())
        }
        let slot = self
            .cache
            .iter_mut()
            .find(|current| current.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(entry);
        Ok(())
    }

    fn job_mut(&mut self, id: JobId) -> Result<&mut Job, Error> {
        self.jobs
            .iter_mut()
            .flatten()
            .find(|job| job.status.id == id)
            .ok_or(Error::JobNotFound)
    }
}

fn is_source_member(root: &str, path: &str) -> bool {
    root == "/" || path.strip_prefix(root).is_some_and(|suffix| suffix.starts_with('/'))
}
