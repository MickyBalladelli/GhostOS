use synos_init::{
    ProcessId, ServiceId, ServiceKind, ServiceSpec, Supervisor, SupervisorError, SupervisorEvent,
    SupervisorRuntime,
};
use synos_pkg::PackageDaemon;
use synos_status::Status;
use synos_synfs::SynFs;
use synos_system_model::{ContentId, LogicalName, RootManifest};
use synos_update::HealthCheck;

use crate::{
    BuildPolicy, CompilerIpcRequest, CompilerIpcResponse, CompilerService, MAX_CACHE_ENTRIES,
    MAX_JOBS, Target,
};

pub const COMPILER_SERVICE_ID: u32 = 0x5255_5354;
pub const COMPILER_SERVICE_NAME: &str = "synos-rustd";
pub const COMPILER_CAPABILITY_PROFILE: u64 = 0x5255_5354_4f46_464c;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompilerBootState {
    Cold,
    Registered,
    Running { process: ProcessId, generation: u32 },
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeCompilerBootConfig {
    pub package: ContentId,
    pub image_id: u128,
    pub target: Target,
    pub policy: BuildPolicy,
}

impl NativeCompilerBootConfig {
    pub const fn new(package: ContentId, image_id: u128, target: Target) -> Self {
        Self {
            package,
            image_id,
            target,
            policy: BuildPolicy::OFFLINE,
        }
    }

    pub const fn with_policy(mut self, policy: BuildPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompilerBootError {
    InvalidConfiguration,
    PackageNotAuthorized,
    AlreadyStarted,
    Supervisor(SupervisorError),
}

impl From<SupervisorError> for CompilerBootError {
    fn from(error: SupervisorError) -> Self {
        Self::Supervisor(error)
    }
}

/// Boot contract for the native compiler service.
///
/// Init owns process creation and restart policy. This type owns the signed
/// package identity and the bounded compiler request state that the process
/// serves after launch.
pub struct CompilerServiceBoot<const JOBS: usize = MAX_JOBS, const CACHE: usize = MAX_CACHE_ENTRIES> {
    config: NativeCompilerBootConfig,
    service: CompilerService<JOBS, CACHE>,
    state: CompilerBootState,
}

impl<const JOBS: usize, const CACHE: usize> CompilerServiceBoot<JOBS, CACHE> {
    fn new(config: NativeCompilerBootConfig) -> Result<Self, CompilerBootError> {
        if config.package.is_zero() || config.image_id == 0 {
            return Err(CompilerBootError::InvalidConfiguration);
        }
        let mut service = CompilerService::new(config.policy);
        service
            .set_toolchain_identity(config.package)
            .map_err(|_| CompilerBootError::InvalidConfiguration)?;
        Ok(Self {
            service,
            config,
            state: CompilerBootState::Cold,
        })
    }

    /// Build the boot state only after the package daemon has verified the
    /// compiler bundle and granted an instantiation receipt.
    pub fn new_authorized<const PACKAGES: usize, const KEYS: usize>(
        packages: &PackageDaemon<PACKAGES, KEYS>,
        config: NativeCompilerBootConfig,
    ) -> Result<Self, CompilerBootError> {
        packages
            .authorize_instantiation(config.package)
            .map_err(|_| CompilerBootError::PackageNotAuthorized)?;
        Self::new(config)
    }

    pub const fn state(&self) -> CompilerBootState {
        self.state
    }

    pub const fn package(&self) -> ContentId {
        self.config.package
    }

    pub const fn target(&self) -> Target {
        self.config.target
    }

    pub const fn service(&self) -> &CompilerService<JOBS, CACHE> {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut CompilerService<JOBS, CACHE> {
        &mut self.service
    }

    /// Ring 3 IPC entry point. Requests are refused until init reports the
    /// signed compiler image as running.
    pub fn handle_ipc(&mut self, request: CompilerIpcRequest) -> CompilerIpcResponse {
        if !matches!(self.state, CompilerBootState::Running { .. }) {
            return CompilerIpcResponse::empty(Status::BUSY);
        }
        if request
            .build
            .is_some_and(|build| build.target != self.config.target)
        {
            return CompilerIpcResponse::empty(Status::INVALID_ARGUMENT);
        }
        self.service.handle_ipc(request)
    }

    pub fn tick(&mut self, now_us: u64) -> usize {
        self.service.tick(now_us)
    }

    /// Register and start the service during SynOS user-space boot.
    pub fn boot<const SERVICES: usize, R: SupervisorRuntime>(
        &mut self,
        supervisor: &mut Supervisor<SERVICES>,
        runtime: &mut R,
    ) -> Result<SupervisorEvent, CompilerBootError> {
        if matches!(self.state, CompilerBootState::Running { .. }) {
            return Err(CompilerBootError::AlreadyStarted);
        }
        if self.state == CompilerBootState::Cold {
            let name = synos_init::ServiceName::new(COMPILER_SERVICE_NAME)
                .map_err(|_| CompilerBootError::InvalidConfiguration)?;
            supervisor.register(ServiceSpec {
                id: ServiceId::new(COMPILER_SERVICE_ID)
                    .ok_or(CompilerBootError::InvalidConfiguration)?,
                name,
                kind: ServiceKind::Compiler,
                image_id: self.config.image_id,
                capability_profile: COMPILER_CAPABILITY_PROFILE,
                restart: synos_init::RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                    .map_err(|_| CompilerBootError::InvalidConfiguration)?,
            })?;
            self.state = CompilerBootState::Registered;
        }

        let event = supervisor.start(
            ServiceId::new(COMPILER_SERVICE_ID)
                .ok_or(CompilerBootError::InvalidConfiguration)?,
            runtime,
        )?;
        if let SupervisorEvent::Started {
            process, generation, ..
        } = event
        {
            self.state = CompilerBootState::Running { process, generation };
        }
        Ok(event)
    }

    pub fn observe(&mut self, status: synos_init::ServiceStatus) {
        self.state = match (status.state, status.process) {
            (synos_init::ServiceState::Running, Some(process)) => CompilerBootState::Running {
                process,
                generation: status.generation,
            },
            (synos_init::ServiceState::Running, None) => CompilerBootState::Unavailable,
            (synos_init::ServiceState::Stopped, _) => CompilerBootState::Registered,
            (synos_init::ServiceState::Backoff, _) | (synos_init::ServiceState::Failed, _) => {
                CompilerBootState::Unavailable
            }
        }
    }

    fn health_status(&self, configuration: &RootManifest) -> Result<(), Status> {
        if !matches!(self.state, CompilerBootState::Running { .. }) {
            return Err(Status::BUSY);
        }
        let name = LogicalName::new(COMPILER_SERVICE_NAME).map_err(|_| Status::INVALID_ARGUMENT)?;
        if configuration.resolve(name) == Some(self.config.package) {
            Ok(())
        } else {
            Err(Status::NOT_FOUND)
        }
    }
}

/// Update health check for the active compiler package.
pub struct CompilerServiceHealthCheck<'a, const JOBS: usize = MAX_JOBS, const CACHE: usize = MAX_CACHE_ENTRIES> {
    boot: &'a CompilerServiceBoot<JOBS, CACHE>,
}

impl<'a, const JOBS: usize, const CACHE: usize> CompilerServiceHealthCheck<'a, JOBS, CACHE> {
    pub const fn new(boot: &'a CompilerServiceBoot<JOBS, CACHE>) -> Self {
        Self { boot }
    }
}

impl<const JOBS: usize, const CACHE: usize> HealthCheck
    for CompilerServiceHealthCheck<'_, JOBS, CACHE>
{
    fn check<const BLOCKS: usize>(
        &mut self,
        _filesystem: &SynFs<BLOCKS>,
        configuration: &RootManifest,
    ) -> Result<(), Status> {
        self.boot.health_status(configuration)
    }
}
