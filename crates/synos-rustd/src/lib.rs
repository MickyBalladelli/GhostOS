#![no_std]
#![forbid(unsafe_code)]

mod boot;
mod design;
mod registry;
mod self_host;
mod source;
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
pub use registry::{
    DependencyPin, DependencyResolution, RegistryEntry, RegistryError, ResolvedDependency,
    SignedLocalRegistry, Version, lockfile_digest, MAX_PACKAGE_NAME_BYTES,
    MAX_REGISTRY_DEPENDENCIES, MAX_REGISTRY_ENTRIES,
};
pub use source::{
    SourceCapability, SourceFile, SourceGrant, SourceSnapshot, MAX_SOURCE_DIRECTORIES,
    MAX_SOURCE_FILES,
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
use synos_synfs::{DirectoryEntry, Error as SynFsError, FileType, SynFs};
use synos_system_model::ContentId;

pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_BINARY_BYTES: usize = 64;
pub const MAX_FEATURES: usize = 16;
pub const MAX_JOBS: usize = 16;
pub const MAX_CACHE_ENTRIES: usize = 32;
pub const MAX_DIAGNOSTIC_BYTES: usize = 512;
pub const MAX_LOG_RECORDS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Capacity,
    InvalidRequest,
    JobNotFound,
    InvalidTransition,
    NetworkDenied,
    ResourceLimit,
    DeadlineExpired,
    SourceCapabilityDenied,
    SourceLimit,
    Registry(RegistryError),
    SourceFilesystem(SynFsError),
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::ResourceLimit => Status::NO_SPACE,
            Self::JobNotFound => Status::NOT_FOUND,
            Self::NetworkDenied => Status::ACCESS_DENIED,
            Self::InvalidRequest
            | Self::InvalidTransition
            | Self::DeadlineExpired
            | Self::SourceCapabilityDenied
            | Self::SourceLimit => {
                Status::INVALID_ARGUMENT
            }
            Self::Registry(error) => match error {
                RegistryError::PackageNotAuthorized => Status::ACCESS_DENIED,
                RegistryError::PackageNotFound | RegistryError::MissingDependency => {
                    Status::NOT_FOUND
                }
                RegistryError::Capacity => Status::NO_SPACE,
                RegistryError::LockfileRequired
                | RegistryError::LockfileMismatch
                | RegistryError::InvalidRegistry
                | RegistryError::Duplicate
                | RegistryError::InvalidDependency => Status::INVALID_ARGUMENT,
            },
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
    isolation: BuildIsolation,
    control: JobControl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheEntry {
    pub source: ContentId,
    pub lockfile: ContentId,
    pub target: Target,
    pub profile: Profile,
    pub result: BuildResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildIsolation {
    pub workspace: Text<MAX_PATH_BYTES>,
    pub scratch: Text<MAX_PATH_BYTES>,
    pub source: ContentId,
    pub lockfile: ContentId,
    pub cache_namespace: ContentId,
    pub limits: ResourceLimits,
    pub deadline_us: u64,
    pub cleanup_pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheKey {
    pub source: ContentId,
    pub lockfile: ContentId,
    pub toolchain: ContentId,
    pub target: Target,
    pub profile: Profile,
    pub features: ContentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentCacheEntry {
    pub key: CacheKey,
    pub result: BuildResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobControl {
    Running,
    StopRequested { at_us: u64 },
    Fenced,
}

pub trait BuildWorkspaceRuntime {
    type Error;

    fn create_workspace(&mut self, isolation: BuildIsolation) -> Result<(), Self::Error>;
    fn cleanup_workspace(&mut self, isolation: BuildIsolation) -> Result<(), Self::Error>;
}

pub struct SynFsWorkspaceRuntime<'a, const BLOCKS: usize> {
    filesystem: &'a mut SynFs<BLOCKS>,
}

impl<'a, const BLOCKS: usize> SynFsWorkspaceRuntime<'a, BLOCKS> {
    pub const fn new(filesystem: &'a mut SynFs<BLOCKS>) -> Self {
        Self { filesystem }
    }
}

impl<const BLOCKS: usize> BuildWorkspaceRuntime for SynFsWorkspaceRuntime<'_, BLOCKS> {
    type Error = SynFsError;

    fn create_workspace(&mut self, isolation: BuildIsolation) -> Result<(), Self::Error> {
        self.filesystem
            .create_directory(isolation.workspace.as_str(), true)?;
        self.filesystem
            .create_directory(isolation.scratch.as_str(), true)?;
        Ok(())
    }

    fn cleanup_workspace(&mut self, isolation: BuildIsolation) -> Result<(), Self::Error> {
        remove_tree(self.filesystem, isolation.workspace.as_str())?;
        remove_tree(self.filesystem, isolation.scratch.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceError<E> {
    Service(Error),
    Runtime(E),
}

pub struct CompilerService<const JOB_CAPACITY: usize = MAX_JOBS, const CACHE_CAPACITY: usize = MAX_CACHE_ENTRIES> {
    policy: BuildPolicy,
    next_id: u64,
    jobs: [Option<Job>; JOB_CAPACITY],
    cache: [Option<CacheEntry>; CACHE_CAPACITY],
    content_cache: [Option<ContentCacheEntry>; CACHE_CAPACITY],
    artifacts: ArtifactSandbox,
    logs: [Option<CompilerLogRecord>; MAX_LOG_RECORDS],
    next_log_sequence: u32,
    toolchain_identity: ContentId,
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
            content_cache: [None; CACHE_CAPACITY],
            artifacts: ArtifactSandbox::new(),
            logs: [None; MAX_LOG_RECORDS],
            next_log_sequence: 1,
            toolchain_identity: ContentId::from_bytes([0; 32]),
        }
    }

    pub fn set_toolchain_identity(&mut self, identity: ContentId) -> Result<(), Error> {
        if identity.is_zero() {
            return Err(Error::InvalidRequest);
        }
        self.toolchain_identity = identity;
        for job in self.jobs.iter_mut().flatten() {
            job.isolation.cache_namespace = cache_namespace(
                job.isolation.source,
                job.isolation.lockfile,
                identity,
                job.status.request,
            );
        }
        Ok(())
    }

    pub fn prepare_job<R: BuildWorkspaceRuntime>(
        &self,
        id: JobId,
        runtime: &mut R,
    ) -> Result<(), WorkspaceError<R::Error>> {
        let isolation = self.isolation(id).map_err(WorkspaceError::Service)?;
        runtime
            .create_workspace(isolation)
            .map_err(WorkspaceError::Runtime)
    }

    pub fn cleanup_job<R: BuildWorkspaceRuntime>(
        &mut self,
        id: JobId,
        runtime: &mut R,
    ) -> Result<(), WorkspaceError<R::Error>> {
        let isolation = self.isolation(id).map_err(WorkspaceError::Service)?;
        runtime
            .cleanup_workspace(isolation)
            .map_err(WorkspaceError::Runtime)?;
        self.artifacts.release_job(id);
        let job = self.job_mut(id).map_err(WorkspaceError::Service)?;
        job.isolation.cleanup_pending = false;
        Ok(())
    }

    pub fn stage_artifact<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &synos_pkg::PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        self.artifacts.stage(packages, artifact)
    }

    pub fn stage_job_artifact<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        id: JobId,
        packages: &synos_pkg::PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        self.status(id).map_err(|_| ArtifactError::NotFound)?;
        self.artifacts.stage_for_job(id, packages, artifact)
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
        self.submit_with_source_identity(request, ContentId::from_bytes([0; 32]))
    }

    fn submit_with_source_identity(
        &mut self,
        request: BuildRequest,
        source_identity: ContentId,
    ) -> Result<JobId, Error> {
        self.policy.authorize(&request)?;
        if !self.jobs.iter().any(Option::is_none) {
            return Err(Error::Capacity);
        }
        let id = JobId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Capacity)?;
        let isolation = build_isolation(id, request, source_identity)?;
        let slot = self
            .jobs
            .iter_mut()
            .find(|job| job.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(Job {
            status: JobStatus {
                id,
                state: JobState::Queued,
                request,
                result: None,
                diagnostic: None,
            },
            started_at_us: 0,
            isolation,
            control: JobControl::Running,
        });
        self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Queued, "build queued");
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

    pub fn submit_from_source<const BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<BLOCKS>,
        grant: SourceGrant,
        request: BuildRequest,
        max_source_bytes: u64,
    ) -> Result<JobId, Error> {
        let snapshot = source::snapshot(
            filesystem,
            grant,
            &request,
            MAX_SOURCE_FILES,
            max_source_bytes,
        )?;
        self.submit_with_source_identity(request, snapshot.identity)
    }

    pub fn submit_from_registry<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        filesystem: &SynFs<BLOCKS>,
        packages: &synos_pkg::PackageDaemon<PACKAGES, KEYS>,
        registry: &SignedLocalRegistry,
        grant: SourceGrant,
        request: BuildRequest,
        pins: &[DependencyPin; MAX_REGISTRY_DEPENDENCIES],
        dependency_count: usize,
        lockfile: ContentId,
        max_source_bytes: u64,
    ) -> Result<JobId, Error> {
        if !request.locked {
            return Err(Error::Registry(RegistryError::LockfileRequired));
        }
        packages
            .authorize_instantiation(registry.package())
            .map_err(|_| Error::Registry(RegistryError::PackageNotAuthorized))?;
        let resolution = registry
            .resolve_locked(lockfile, pins, dependency_count)
            .map_err(Error::Registry)?;
        for dependency in resolution.iter() {
            packages
                .authorize_instantiation(dependency.pin.package)
                .map_err(|_| Error::Registry(RegistryError::PackageNotAuthorized))?;
        }
        let snapshot = source::snapshot(
            filesystem,
            grant,
            &request,
            MAX_SOURCE_FILES,
            max_source_bytes,
        )?;
        let id = self.submit_with_source_identity(request, snapshot.identity)?;
        self.set_lockfile(id, lockfile)?;
        Ok(id)
    }

    fn set_lockfile(&mut self, id: JobId, lockfile: ContentId) -> Result<(), Error> {
        let toolchain_identity = self.toolchain_identity;
        let job = self.job_mut(id)?;
        job.isolation.lockfile = lockfile;
        job.isolation.cache_namespace = cache_namespace(
            job.isolation.source,
            lockfile,
            toolchain_identity,
            job.status.request,
        );
        Ok(())
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
        self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Started, "build started");
        Ok(())
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        let mut expired_ids = [None; JOB_CAPACITY];
        let mut expired_length = 0;
        for job in self.jobs.iter_mut().flatten() {
            if job.status.state == JobState::Running
                && job.started_at_us != 0
                && now_us.saturating_sub(job.started_at_us) >= job.status.request.limits.deadline_us
            {
                job.status.state = JobState::Failed;
                job.control = JobControl::Fenced;
                job.isolation.cleanup_pending = true;
                job.status.diagnostic = Some(Diagnostic {
                    code: 408,
                    message: Text::new("build deadline expired")
                        .expect("static diagnostic fits fixed buffer"),
                });
                if expired_length < JOB_CAPACITY {
                    expired_ids[expired_length] = Some(job.status.id);
                    expired_length += 1;
                }
                expired += 1;
            }
        }
        for id in expired_ids.into_iter().take(expired_length).flatten() {
            self.emit_log(id, CompilerLogLevel::Error, CompilerEventKind::Failed, "build deadline expired");
        }
        expired
    }

    pub fn request_cancel(&mut self, id: JobId, now_us: u64) -> Result<CancellationDisposition, Error> {
        let mut queued = false;
        let mut requested = false;
        let mut fenced = false;
        let disposition = {
            let job = self.job_mut(id)?;
            match job.status.state {
                JobState::Queued => {
                    job.status.state = JobState::Cancelled;
                    job.control = JobControl::Fenced;
                    job.isolation.cleanup_pending = true;
                    queued = true;
                    CancellationDisposition::CancelQueued
                }
                JobState::Running => {
                    let at_us = match job.control {
                        JobControl::StopRequested { at_us } => at_us,
                        _ => {
                            job.control = JobControl::StopRequested { at_us: now_us };
                            requested = true;
                            now_us
                        }
                    };
                    if now_us.saturating_sub(at_us) >= CANCELLATION_GRACE_US {
                        job.status.state = JobState::Cancelled;
                        job.control = JobControl::Fenced;
                        job.isolation.cleanup_pending = true;
                        fenced = true;
                    }
                    CancellationDisposition::RequestCooperativeStop
                }
                JobState::Completed | JobState::Failed | JobState::Cancelled => {
                    CancellationDisposition::AlreadyTerminal
                }
            }
        };
        if queued {
            self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Cancelled, "queued build cancelled");
        } else if requested {
            self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Progress, "cooperative stop requested");
        } else if fenced {
            self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Cancelled, "build fenced and cancelled");
        }
        Ok(disposition)
    }

    pub fn tick(&mut self, now_us: u64) -> usize {
        let expired = self.expire(now_us);
        let mut ids = [None; JOB_CAPACITY];
        let mut length = 0;
        for job in self.jobs.iter().flatten() {
            if matches!(job.control, JobControl::StopRequested { at_us } if now_us.saturating_sub(at_us) >= CANCELLATION_GRACE_US)
                && length < JOB_CAPACITY
            {
                ids[length] = Some(job.status.id);
                length += 1;
            }
        }
        let mut fenced = 0;
        for id in ids.into_iter().take(length).flatten() {
            if self.request_cancel(id, now_us).is_ok() {
                fenced += 1;
            }
        }
        expired + fenced
    }

    pub fn complete(&mut self, id: JobId, result: BuildResult) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if job.status.state != JobState::Running {
            return Err(Error::InvalidTransition);
        }
        if result.target != job.status.request.target || job.control != JobControl::Running {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Completed;
        job.status.result = Some(result);
        job.isolation.cleanup_pending = true;
        self.emit_log(id, CompilerLogLevel::Info, CompilerEventKind::Completed, "build completed");
        Ok(())
    }

    pub fn fail(&mut self, id: JobId, diagnostic: Diagnostic) -> Result<(), Error> {
        let job = self.job_mut(id)?;
        if !matches!(job.status.state, JobState::Queued | JobState::Running) {
            return Err(Error::InvalidTransition);
        }
        job.status.state = JobState::Failed;
        job.status.diagnostic = Some(diagnostic);
        job.isolation.cleanup_pending = true;
        self.emit_log(id, CompilerLogLevel::Error, CompilerEventKind::Failed, diagnostic.message.as_str());
        Ok(())
    }

    pub fn cancel(&mut self, id: JobId) -> Result<(), Error> {
        self.request_cancel(id, 0).map(|_| ())
    }

    pub fn status(&self, id: JobId) -> Result<JobStatus, Error> {
        self.jobs
            .iter()
            .flatten()
            .find(|job| job.status.id == id)
            .map(|job| job.status)
            .ok_or(Error::JobNotFound)
    }

    pub fn isolation(&self, id: JobId) -> Result<BuildIsolation, Error> {
        self.jobs
            .iter()
            .flatten()
            .find(|job| job.status.id == id)
            .map(|job| job.isolation)
            .ok_or(Error::JobNotFound)
    }

    pub fn release(&mut self, id: JobId) -> Result<(), Error> {
        let slot = self
            .jobs
            .iter_mut()
            .find(|job| job.is_some_and(|job| job.status.id == id))
            .ok_or(Error::JobNotFound)?;
        *slot = None;
        self.artifacts.release_job(id);
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

    pub fn cache_key(
        &self,
        source: ContentId,
        lockfile: ContentId,
        request: BuildRequest,
    ) -> CacheKey {
        CacheKey {
            source,
            lockfile,
            toolchain: self.toolchain_identity,
            target: request.target,
            profile: request.profile,
            features: feature_digest(request),
        }
    }

    pub fn cache_lookup_key(&self, key: CacheKey) -> Option<BuildResult> {
        self.content_cache
            .iter()
            .flatten()
            .find(|entry| entry.key == key)
            .map(|entry| entry.result)
    }

    pub fn cache_insert_key(&mut self, entry: ContentCacheEntry) -> Result<(), Error> {
        if entry.key.source.is_zero()
            || entry.key.lockfile.is_zero()
            || entry.key.toolchain.is_zero()
            || entry.result.package.is_zero()
            || entry.result.payload.is_zero()
            || entry.key.target != entry.result.target
        {
            return Err(Error::InvalidRequest);
        }
        if let Some(existing) = self
            .content_cache
            .iter_mut()
            .flatten()
            .find(|current| current.key == entry.key)
        {
            *existing = entry;
            return Ok(())
        }
        let slot = self
            .content_cache
            .iter_mut()
            .find(|current| current.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(entry);
        Ok(())
    }

    pub fn complete_with_cache(
        &mut self,
        id: JobId,
        result: BuildResult,
    ) -> Result<(), Error> {
        let (request, isolation) = {
            let job = self.job_mut(id)?;
            (job.status.request, job.isolation)
        };
        self.complete(id, result)?;
        if !isolation.source.is_zero()
            && !isolation.lockfile.is_zero()
            && !self.toolchain_identity.is_zero()
        {
            let key = self.cache_key(isolation.source, isolation.lockfile, request);
            self.cache_insert_key(ContentCacheEntry { key, result })?;
        }
        Ok(())
    }

    pub fn read_log(
        &self,
        id: JobId,
        after_sequence: u32,
    ) -> Result<Option<CompilerLogRecord>, Error> {
        self.status(id)?;
        Ok(self
            .logs
            .iter()
            .flatten()
            .filter(|record| record.job == id.raw() && record.sequence > after_sequence)
            .min_by_key(|record| record.sequence)
            .copied())
    }

    pub fn handle_ipc(&mut self, request: CompilerIpcRequest) -> CompilerIpcResponse {
        if let Err(error) = request.validate() {
            return CompilerIpcResponse::empty(error.status());
        }
        match request.operation {
            CompilerOperation::Submit => {
                let Some(build) = request.build else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                match self.submit(build) {
                    Ok(id) => CompilerIpcResponse {
                        job: Some(id),
                        state: Some(JobState::Queued),
                        event: Some(CompilerEventKind::Queued),
                        sequence: self.next_log_sequence.saturating_sub(1),
                        ..CompilerIpcResponse::empty(Status::NORMAL)
                    },
                    Err(error) => CompilerIpcResponse::empty(error.status()),
                }
            }
            CompilerOperation::Start => {
                let Some(id) = JobId::from_raw(request.job) else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                match self.start_at(id, request.now_us) {
                    Ok(()) => self.response_for(id, Status::NORMAL, request.log_sequence),
                    Err(error) => CompilerIpcResponse::empty(error.status()),
                }
            }
            CompilerOperation::Poll => {
                let Some(id) = JobId::from_raw(request.job) else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                self.response_for(id, Status::NORMAL, request.log_sequence)
            }
            CompilerOperation::Cancel => {
                let Some(id) = JobId::from_raw(request.job) else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                match self.request_cancel(id, request.now_us) {
                    Ok(_) => self.response_for(id, Status::NORMAL, request.log_sequence),
                    Err(error) => CompilerIpcResponse::empty(error.status()),
                }
            }
            CompilerOperation::Release => {
                let Some(id) = JobId::from_raw(request.job) else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                match self.release(id) {
                    Ok(()) => CompilerIpcResponse::empty(Status::NORMAL),
                    Err(error) => CompilerIpcResponse::empty(error.status()),
                }
            }
            CompilerOperation::ReadLog => {
                let Some(id) = JobId::from_raw(request.job) else {
                    return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
                };
                match self.read_log(id, request.log_sequence) {
                    Ok(Some(log)) => CompilerIpcResponse {
                        job: Some(id),
                        event: Some(log.event),
                        sequence: log.sequence,
                        log: Some(log),
                        ..CompilerIpcResponse::empty(log.event.status())
                    },
                    Ok(None) => CompilerIpcResponse {
                        job: Some(id),
                        sequence: request.log_sequence,
                        ..CompilerIpcResponse::empty(Status::PENDING)
                    },
                    Err(error) => CompilerIpcResponse::empty(error.status()),
                }
            }
        }
    }

    fn response_for(
        &self,
        id: JobId,
        status: Status,
        after_sequence: u32,
    ) -> CompilerIpcResponse {
        let Ok(job) = self.status(id) else {
            return CompilerIpcResponse::empty(Status::NOT_FOUND);
        };
        let log = self.read_log(id, after_sequence).ok().flatten();
        CompilerIpcResponse {
            job: Some(id),
            state: Some(job.state),
            event: log.map(|record| record.event),
            sequence: log.map_or(after_sequence, |record| record.sequence),
            diagnostic: job.diagnostic,
            result: job.result,
            log,
            ..CompilerIpcResponse::empty(status)
        }
    }

    fn emit_log(&mut self, id: JobId, level: CompilerLogLevel, event: CompilerEventKind, message: &str) {
        if MAX_LOG_RECORDS == 0 {
            return
        }
        let Ok(message) = Text::new(message) else {
            return;
        };
        let sequence = self.next_log_sequence;
        self.next_log_sequence = self.next_log_sequence.wrapping_add(1).max(1);
        self.logs[(sequence as usize) % MAX_LOG_RECORDS] = Some(CompilerLogRecord {
            job: id.raw(),
            sequence,
            level,
            event,
            message,
        });
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

fn build_isolation(
    id: JobId,
    request: BuildRequest,
    source: ContentId,
) -> Result<BuildIsolation, Error> {
    Ok(BuildIsolation {
        workspace: path_with_id(SYNFS_BUILD_STATE, id.raw())?,
        scratch: path_with_id(SYNFS_TEMP, id.raw())?,
        source,
        lockfile: ContentId::from_bytes([0; 32]),
        cache_namespace: cache_namespace(
            source,
            ContentId::from_bytes([0; 32]),
            ContentId::from_bytes([0; 32]),
            request,
        ),
        limits: request.limits,
        deadline_us: request.limits.deadline_us,
        cleanup_pending: false,
    })
}

fn path_with_id(prefix: &str, id: u64) -> Result<Text<MAX_PATH_BYTES>, Error> {
    let mut bytes = [0; MAX_PATH_BYTES];
    if prefix.len() + 1 >= MAX_PATH_BYTES {
        return Err(Error::InvalidRequest);
    }
    bytes[..prefix.len()].copy_from_slice(prefix.as_bytes());
    let mut cursor = prefix.len();
    bytes[cursor] = b'/';
    cursor += 1;
    let mut digits = [0; 20];
    let mut length = 0;
    let mut value = id;
    loop {
        digits[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
        if value == 0 {
            break
        }
    }
    for digit in digits[..length].iter().rev() {
        bytes[cursor] = *digit;
        cursor += 1;
    }
    Text::new(core::str::from_utf8(&bytes[..cursor]).map_err(|_| Error::InvalidRequest)?)
}

fn feature_digest(request: BuildRequest) -> ContentId {
    let mut material = [0; MAX_FEATURES * (MAX_BINARY_BYTES + 1)];
    let mut cursor = 0;
    for feature in request.features.iter().flatten() {
        let bytes = feature.as_str().as_bytes();
        if cursor + bytes.len() + 1 > material.len() {
            break
        }
        material[cursor..cursor + bytes.len()].copy_from_slice(bytes);
        cursor += bytes.len();
        material[cursor] = 0;
        cursor += 1;
    }
    ContentId::hash(&material[..cursor])
}

fn cache_namespace(
    source: ContentId,
    lockfile: ContentId,
    toolchain: ContentId,
    request: BuildRequest,
) -> ContentId {
    let mut material = [0; 32 * 3 + 3 + 32];
    let mut cursor = 0;
    for value in [source, lockfile, toolchain] {
        material[cursor..cursor + 32].copy_from_slice(value.as_bytes());
        cursor += 32;
    }
    material[cursor] = request.target as u8;
    cursor += 1;
    material[cursor] = request.profile as u8;
    cursor += 1;
    material[cursor] = request.locked as u8;
    cursor += 1;
    material[cursor..cursor + 32].copy_from_slice(feature_digest(request).as_bytes());
    ContentId::hash(&material[..cursor + 32])
}

fn remove_tree<const BLOCKS: usize>(
    filesystem: &mut SynFs<BLOCKS>,
    path: &str,
) -> Result<(), SynFsError> {
    let metadata = match filesystem.lookup(path) {
        Ok(metadata) => metadata,
        Err(SynFsError::NotFound) => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type != FileType::Directory {
        filesystem.delete(path)?;
        return Ok(())
    }
    let mut entries = [DirectoryEntry::EMPTY; 256];
    let count = filesystem.list_directory(path, &mut entries)?;
    for entry in entries.into_iter().take(count) {
        let child = workspace_child(path, entry.name.as_str())?;
        remove_tree(filesystem, child.as_str())?;
    }
    filesystem.remove_directory(path)?;
    Ok(())
}

fn workspace_child(root: &str, name: &str) -> Result<Text<MAX_PATH_BYTES>, SynFsError> {
    let separator = usize::from(!root.ends_with('/'));
    let length = root
        .len()
        .checked_add(separator)
        .and_then(|length| length.checked_add(name.len()))
        .ok_or(SynFsError::InvalidPath)?;
    if length > MAX_PATH_BYTES {
        return Err(SynFsError::InvalidPath);
    }
    let mut bytes = [0; MAX_PATH_BYTES];
    bytes[..root.len()].copy_from_slice(root.as_bytes());
    let mut cursor = root.len();
    if separator != 0 {
        bytes[cursor] = b'/';
        cursor += 1;
    }
    bytes[cursor..length].copy_from_slice(name.as_bytes());
    Text::new(core::str::from_utf8(&bytes[..length]).map_err(|_| SynFsError::InvalidPath)?)
        .map_err(|_| SynFsError::InvalidPath)
}
