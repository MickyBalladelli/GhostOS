use synos_init::{CrashReason, ExitReason, ProcessId};
use synos_pkg::{InstantiationReceipt, PackageDaemon, PackageError};
use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::{
    AppManifest, ApplicationKind, BoundedText, CapabilityKind, CapabilityRequest, CapabilityRights,
    MAX_APP_CAPABILITIES, MAX_RESOURCE_NAME_BYTES, ManifestError, Placement, RestartMode,
};

pub const DEFAULT_APPLICATION_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ApplicationId(u32);

impl ApplicationId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRule {
    pub resource: BoundedText<MAX_RESOURCE_NAME_BYTES>,
    pub kind: CapabilityKind,
    pub maximum_rights: CapabilityRights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    Capacity,
    DuplicateResource,
    InvalidRule,
    MissingRequiredCapability,
    RightsEscalation,
    WrongResourceKind,
}

impl IntoStatus for PolicyError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::MissingRequiredCapability => Status::NOT_FOUND,
            Self::RightsEscalation | Self::WrongResourceKind => Status::ACCESS_DENIED,
            Self::DuplicateResource | Self::InvalidRule => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy)]
pub struct CapabilityPolicy<const CAPACITY: usize = MAX_APP_CAPABILITIES> {
    rules: [Option<CapabilityRule>; CAPACITY],
}

impl<const CAPACITY: usize> CapabilityPolicy<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            rules: [None; CAPACITY],
        }
    }

    pub fn allow(&mut self, rule: CapabilityRule) -> Result<(), PolicyError> {
        if rule.resource.is_empty() || rule.maximum_rights.is_empty() {
            return Err(PolicyError::InvalidRule);
        }
        if self
            .rules
            .iter()
            .flatten()
            .any(|existing| existing.resource == rule.resource)
        {
            return Err(PolicyError::DuplicateResource);
        }
        let slot = self
            .rules
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PolicyError::Capacity)?;
        *slot = Some(rule);
        Ok(())
    }

    pub fn authorize(&self, manifest: &AppManifest) -> Result<CapabilitySet, PolicyError> {
        let mut approved = CapabilitySet::new();
        for request in manifest.capabilities() {
            let Some(rule) = self
                .rules
                .iter()
                .flatten()
                .find(|rule| rule.resource == request.resource)
            else {
                if request.required {
                    return Err(PolicyError::MissingRequiredCapability);
                }
                continue;
            };
            if rule.kind != request.kind {
                return Err(PolicyError::WrongResourceKind);
            }
            if !rule.maximum_rights.contains(request.rights) {
                return Err(PolicyError::RightsEscalation);
            }
            approved.push(request)?
        }
        Ok(approved)
    }
}

impl<const CAPACITY: usize> Default for CapabilityPolicy<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySet {
    capabilities: [Option<CapabilityRequest>; MAX_APP_CAPABILITIES],
}

impl CapabilitySet {
    const fn new() -> Self {
        Self {
            capabilities: [None; MAX_APP_CAPABILITIES],
        }
    }

    fn push(&mut self, capability: CapabilityRequest) -> Result<(), PolicyError> {
        let slot = self
            .capabilities
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PolicyError::Capacity)?;
        *slot = Some(capability);
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = CapabilityRequest> + '_ {
        self.capabilities.iter().flatten().copied()
    }

    pub fn as_slice(&self) -> &[Option<CapabilityRequest>; MAX_APP_CAPABILITIES] {
        &self.capabilities
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationState {
    Stopped,
    Running,
    Backoff,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppSpawnRequest<'a> {
    pub application: ApplicationId,
    pub name: &'a str,
    pub image: u128,
    pub kind: ApplicationKind,
    pub placement: Placement,
    pub generation: u32,
    pub capabilities: &'a [Option<CapabilityRequest>; MAX_APP_CAPABILITIES],
    pub executable: Option<ExecutableImage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutableImage {
    pub package: ContentId,
    pub payload: ContentId,
    pub entry_offset: u64,
    pub byte_length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageLaunchError {
    InvalidImage,
    Package(PackageError),
    Supervisor(SupervisorError),
}

impl IntoStatus for PackageLaunchError {
    fn status(self) -> Status {
        match self {
            Self::InvalidImage => Status::INVALID_ARGUMENT,
            Self::Package(error) => error.status(),
            Self::Supervisor(error) => error.status(),
        }
    }
}

impl From<PackageError> for PackageLaunchError {
    fn from(error: PackageError) -> Self {
        Self::Package(error)
    }
}

impl From<SupervisorError> for PackageLaunchError {
    fn from(error: SupervisorError) -> Self {
        Self::Supervisor(error)
    }
}

pub trait ApplicationRuntime {
    type Error;

    fn spawn(&mut self, request: AppSpawnRequest<'_>) -> Result<ProcessId, Self::Error>;
    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationEvent {
    Started {
        application: ApplicationId,
        process: ProcessId,
        generation: u32,
    },
    RestartScheduled {
        application: ApplicationId,
        at_us: u64,
    },
    Stopped {
        application: ApplicationId,
    },
    Failed {
        application: ApplicationId,
        reason: Option<CrashReason>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationStatus {
    pub application: ApplicationId,
    pub state: ApplicationState,
    pub process: Option<ProcessId>,
    pub generation: u32,
    pub restart_count: u16,
    pub restart_at_us: u64,
    pub last_exit: Option<ExitReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    AlreadyRegistered,
    AlreadyRunning,
    Capacity,
    FenceFailed,
    InvalidManifest(ManifestError),
    NotFound,
    Policy(PolicyError),
    SpawnFailed,
    StaleExit,
}

impl IntoStatus for SupervisorError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::NotFound | Self::StaleExit => Status::NOT_FOUND,
            Self::Policy(error) => error.status(),
            Self::InvalidManifest(error) => error.status(),
            Self::AlreadyRegistered | Self::AlreadyRunning => Status::INVALID_ARGUMENT,
            Self::FenceFailed | Self::SpawnFailed => Status::BUSY,
        }
    }
}

#[derive(Clone, Copy)]
struct ApplicationSlot {
    occupied: bool,
    application: ApplicationId,
    manifest: AppManifest,
    capabilities: CapabilitySet,
    state: ApplicationState,
    process: Option<ProcessId>,
    generation: u32,
    restart_count: u16,
    window_started_us: u64,
    restart_at_us: u64,
    last_exit: Option<ExitReason>,
    executable: Option<ExecutableImage>,
}

impl ApplicationSlot {
    const EMPTY: Self = Self {
        occupied: false,
        application: ApplicationId(0),
        manifest: AppManifest {
            schema: 0,
            name: BoundedText::EMPTY,
            image: 0,
            kind: ApplicationKind::Service,
            runtime: crate::RuntimeSpec {
                placement: Placement::Local,
                restart_mode: RestartMode::Never,
                restart: synos_init::RestartPolicy::NEVER,
            },
            capabilities: [CapabilityRequest::EMPTY; MAX_APP_CAPABILITIES],
            capability_count: 0,
        },
        capabilities: CapabilitySet::new(),
        state: ApplicationState::Stopped,
        process: None,
        generation: 0,
        restart_count: 0,
        window_started_us: 0,
        restart_at_us: 0,
        last_exit: None,
        executable: None,
    };
}

pub struct ApplicationSupervisor<const CAPACITY: usize = DEFAULT_APPLICATION_CAPACITY> {
    applications: [ApplicationSlot; CAPACITY],
}

impl<const CAPACITY: usize> ApplicationSupervisor<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            applications: [ApplicationSlot::EMPTY; CAPACITY],
        }
    }

    pub fn register<const RULES: usize>(
        &mut self,
        application: ApplicationId,
        manifest: AppManifest,
        policy: &CapabilityPolicy<RULES>,
    ) -> Result<(), SupervisorError> {
        if manifest.schema != crate::APP_MANIFEST_SCHEMA
            || manifest.name.is_empty()
            || manifest.image == 0
        {
            return Err(SupervisorError::InvalidManifest(
                ManifestError::InvalidManifest,
            ));
        }
        if self
            .applications
            .iter()
            .any(|slot| slot.occupied && slot.application == application)
        {
            return Err(SupervisorError::AlreadyRegistered);
        }
        let capabilities = policy
            .authorize(&manifest)
            .map_err(SupervisorError::Policy)?;
        let slot = self
            .applications
            .iter_mut()
            .find(|slot| !slot.occupied)
            .ok_or(SupervisorError::Capacity)?;
        *slot = ApplicationSlot {
            occupied: true,
            application,
            manifest,
            capabilities,
            ..ApplicationSlot::EMPTY
        };
        Ok(())
    }

    /// Register an application after its signed package has been checked.
    /// The package identity and entry point stay attached to the slot until
    /// the process runtime receives the launch request.
    pub fn register_package<const RULES: usize>(
        &mut self,
        application: ApplicationId,
        manifest: AppManifest,
        executable: ExecutableImage,
        policy: &CapabilityPolicy<RULES>,
    ) -> Result<(), SupervisorError> {
        if executable.package.is_zero()
            || executable.payload.is_zero()
            || executable.entry_offset >= executable.byte_length
        {
            return Err(SupervisorError::InvalidManifest(ManifestError::InvalidManifest));
        }
        self.register(application, manifest, policy)?;
        self.slot_mut(application)?.executable = Some(executable);
        Ok(())
    }

    /// Verify a package receipt, validate its entry point, apply capability
    /// policy, and launch the verified package through the supervisor.
    pub fn launch_package<
        const RULES: usize,
        const PACKAGES: usize,
        const KEYS: usize,
        R: ApplicationRuntime,
    >(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        application: ApplicationId,
        manifest: AppManifest,
        receipt: InstantiationReceipt,
        policy: &CapabilityPolicy<RULES>,
        runtime: &mut R,
    ) -> Result<ApplicationEvent, PackageLaunchError> {
        packages.validate_instantiation(receipt)?;
        let package = packages
            .manifest(receipt.package())
            .ok_or(PackageError::InstantiationDenied)?;
        if package.content != receipt.package()
            || package.payload.is_zero()
            || package.entry_offset >= package.byte_length
        {
            return Err(PackageLaunchError::InvalidImage);
        }
        self.register_package(
            application,
            manifest,
            ExecutableImage {
                package: package.content,
                payload: package.payload,
                entry_offset: package.entry_offset,
                byte_length: package.byte_length,
            },
            policy,
        )?;
        Ok(self.start(application, runtime)?)
    }

    pub fn start<R: ApplicationRuntime>(
        &mut self,
        application: ApplicationId,
        runtime: &mut R,
    ) -> Result<ApplicationEvent, SupervisorError> {
        let slot = self.slot_mut(application)?;
        if slot.state == ApplicationState::Running {
            return Err(SupervisorError::AlreadyRunning);
        }
        spawn(slot, runtime)
    }

    pub fn report_exit<R: ApplicationRuntime>(
        &mut self,
        process: ProcessId,
        reason: ExitReason,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<ApplicationEvent, SupervisorError> {
        let slot = self
            .applications
            .iter_mut()
            .find(|slot| slot.occupied && slot.process == Some(process))
            .ok_or(SupervisorError::StaleExit)?;
        runtime
            .fence_process(process)
            .map_err(|_| SupervisorError::FenceFailed)?;
        slot.process = None;
        slot.last_exit = Some(reason);

        let should_restart = match (slot.manifest.runtime.restart_mode, reason) {
            (RestartMode::Always, _) => true,
            (RestartMode::OnFailure, ExitReason::Crash(_)) => true,
            _ => false,
        };
        if !should_restart {
            slot.state = ApplicationState::Stopped;
            return Ok(ApplicationEvent::Stopped {
                application: slot.application,
            });
        }

        let policy = slot.manifest.runtime.restart;
        if now_us.saturating_sub(slot.window_started_us) >= policy.window_us {
            slot.window_started_us = now_us;
            slot.restart_count = 0
        }
        if slot.restart_count >= policy.max_restarts {
            slot.state = ApplicationState::Failed;
            return Ok(ApplicationEvent::Failed {
                application: slot.application,
                reason: match reason {
                    ExitReason::Crash(reason) => Some(reason),
                    ExitReason::Clean => None,
                },
            });
        }

        let shift = core::cmp::min(slot.restart_count as u32, 63);
        let backoff = policy
            .initial_backoff_us
            .saturating_mul(1u64 << shift)
            .min(policy.max_backoff_us);
        slot.restart_count += 1;
        slot.restart_at_us = now_us.saturating_add(backoff);
        slot.state = ApplicationState::Backoff;
        Ok(ApplicationEvent::RestartScheduled {
            application: slot.application,
            at_us: slot.restart_at_us,
        })
    }

    pub fn tick<R: ApplicationRuntime>(
        &mut self,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<Option<ApplicationEvent>, SupervisorError> {
        let Some(slot) = self.applications.iter_mut().find(|slot| {
            slot.occupied && slot.state == ApplicationState::Backoff && now_us >= slot.restart_at_us
        }) else {
            return Ok(None);
        };
        spawn(slot, runtime).map(Some)
    }

    pub fn status(&self, application: ApplicationId) -> Result<ApplicationStatus, SupervisorError> {
        let slot = self
            .applications
            .iter()
            .find(|slot| slot.occupied && slot.application == application)
            .ok_or(SupervisorError::NotFound)?;
        Ok(status(slot))
    }

    pub fn applications(&self) -> impl Iterator<Item = ApplicationStatus> + '_ {
        self.applications
            .iter()
            .filter(|slot| slot.occupied)
            .map(status)
    }

    fn slot_mut(
        &mut self,
        application: ApplicationId,
    ) -> Result<&mut ApplicationSlot, SupervisorError> {
        self.applications
            .iter_mut()
            .find(|slot| slot.occupied && slot.application == application)
            .ok_or(SupervisorError::NotFound)
    }
}

impl<const CAPACITY: usize> Default for ApplicationSupervisor<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn status(slot: &ApplicationSlot) -> ApplicationStatus {
    ApplicationStatus {
        application: slot.application,
        state: slot.state,
        process: slot.process,
        generation: slot.generation,
        restart_count: slot.restart_count,
        restart_at_us: slot.restart_at_us,
        last_exit: slot.last_exit,
    }
}

fn spawn<R: ApplicationRuntime>(
    slot: &mut ApplicationSlot,
    runtime: &mut R,
) -> Result<ApplicationEvent, SupervisorError> {
    let generation = slot.generation.wrapping_add(1).max(1);
    let process = runtime
        .spawn(AppSpawnRequest {
            application: slot.application,
            name: slot.manifest.name.as_str(),
            image: slot.manifest.image,
            kind: slot.manifest.kind,
            placement: slot.manifest.runtime.placement,
            generation,
            capabilities: slot.capabilities.as_slice(),
            executable: slot.executable,
        })
        .map_err(|_| SupervisorError::SpawnFailed)?;
    slot.generation = generation;
    slot.process = Some(process);
    slot.state = ApplicationState::Running;
    Ok(ApplicationEvent::Started {
        application: slot.application,
        process,
        generation,
    })
}
