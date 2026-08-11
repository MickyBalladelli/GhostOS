#![no_std]
#![forbid(unsafe_code)]

use synos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use synos_status::{IntoStatus, Severity, Status, facility};

pub const DEFAULT_SERVICE_CAPACITY: usize = 32;
pub const MAX_SERVICE_NAME_BYTES: usize = 48;
pub const MAX_SERVICE_DEPENDENCIES: usize = DEFAULT_SERVICE_CAPACITY;
pub const LIFECYCLE_TRACE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ServiceId(u32);

impl ServiceId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceName {
    bytes: [u8; MAX_SERVICE_NAME_BYTES],
    len: u8,
}

impl ServiceName {
    pub fn new(name: &str) -> Result<Self, SupervisorError> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_SERVICE_NAME_BYTES || bytes.contains(&0) {
            return Err(SupervisorError::InvalidService);
        }
        let mut stored = [0; MAX_SERVICE_NAME_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceKind {
    StorageDriver,
    NetworkDriver,
    AcceleratorDriver,
    Compiler,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartPolicy {
    pub max_restarts: u16,
    pub window_us: u64,
    pub initial_backoff_us: u64,
    pub max_backoff_us: u64,
}

impl RestartPolicy {
    pub const NEVER: Self = Self {
        max_restarts: 0,
        window_us: 1,
        initial_backoff_us: 0,
        max_backoff_us: 0,
    };

    pub const fn on_failure(
        max_restarts: u16,
        window_us: u64,
        initial_backoff_us: u64,
        max_backoff_us: u64,
    ) -> Result<Self, SupervisorError> {
        if max_restarts == 0
            || window_us == 0
            || initial_backoff_us == 0
            || max_backoff_us < initial_backoff_us
        {
            Err(SupervisorError::InvalidPolicy)
        } else {
            Ok(Self {
                max_restarts,
                window_us,
                initial_backoff_us,
                max_backoff_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceSpec {
    pub id: ServiceId,
    pub name: ServiceName,
    pub kind: ServiceKind,
    pub image_id: u128,
    pub capability_profile: u64,
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnRequest {
    pub service: ServiceId,
    pub image_id: u128,
    pub capability_profile: u64,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrashReason {
    Panic,
    ProtectionFault,
    IllegalInstruction,
    Watchdog,
    UnexpectedExit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    Clean,
    Crash(CrashReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Stopped,
    Running,
    Backoff,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceStatus {
    pub spec: ServiceSpec,
    pub state: ServiceState,
    pub readiness: ServiceReadiness,
    pub process: Option<ProcessId>,
    pub generation: u32,
    pub restart_count: u16,
    pub restart_at_us: u64,
    pub last_exit: Option<ExitReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceReadiness {
    Waiting,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEvent {
    BootStarted,
    ServiceStarted { service: ServiceId, generation: u32 },
    ServiceReady { service: ServiceId },
    BootCompleted,
    ShutdownStarted,
    ServiceStopped { service: ServiceId },
    ShutdownCompleted,
    RebootStarted,
    RebootCompleted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleTrace {
    events: [Option<LifecycleEvent>; LIFECYCLE_TRACE_CAPACITY],
    len: usize,
}

impl LifecycleTrace {
    pub const fn new() -> Self {
        Self {
            events: [None; LIFECYCLE_TRACE_CAPACITY],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn events(&self) -> impl Iterator<Item = LifecycleEvent> + '_ {
        self.events[..self.len].iter().flatten().copied()
    }

    fn push(&mut self, event: LifecycleEvent) -> Result<(), SupervisorError> {
        let slot = self
            .events
            .get_mut(self.len)
            .ok_or(SupervisorError::TraceFull)?;
        *slot = Some(event);
        self.len += 1;
        Ok(())
    }
}

impl Default for LifecycleTrace {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorEvent {
    Started {
        service: ServiceId,
        process: ProcessId,
        generation: u32,
    },
    RestartScheduled {
        service: ServiceId,
        at_us: u64,
        reason: CrashReason,
    },
    Stopped {
        service: ServiceId,
    },
    Failed {
        service: ServiceId,
        reason: CrashReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    AlreadyRegistered,
    Capacity,
    FenceFailed,
    InvalidPolicy,
    InvalidService,
    NotFound,
    SpawnFailed,
    StaleExit,
    Interrupted,
    MissingDependency,
    DependencyCycle,
    DependencyNotReady,
    NotReady,
    ServiceFailed,
    TraceFull,
}

impl IntoStatus for SupervisorError {
    fn status(self) -> Status {
        match self {
            Self::AlreadyRegistered | Self::InvalidPolicy | Self::InvalidService => {
                Status::INVALID_ARGUMENT
            }
            Self::Capacity => Status::NO_SPACE,
            Self::NotFound | Self::StaleExit | Self::MissingDependency => Status::NOT_FOUND,
            Self::DependencyCycle | Self::DependencyNotReady | Self::NotReady | Self::ServiceFailed => {
                Status::BUSY
            }
            Self::FenceFailed | Self::SpawnFailed => {
                Status::new(Severity::Error, facility::DRIVER, 1, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::Interrupted | Self::TraceFull => Status::BUSY,
        }
    }
}

/// Ring 0 operations used by the Ring 3 supervisor.
///
/// `fence_process` revokes the dead process' capabilities, IPC endpoints, DMA
/// mappings, and interrupts before its replacement is spawned.
pub trait SupervisorRuntime {
    type Error;

    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessId, Self::Error>;
    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy)]
struct ServiceSlot {
    occupied: bool,
    spec: ServiceSpec,
    dependencies: [Option<ServiceId>; MAX_SERVICE_DEPENDENCIES],
    dependency_count: usize,
    state: ServiceState,
    ready: bool,
    process: Option<ProcessId>,
    generation: u32,
    restart_count: u16,
    window_started_us: u64,
    restart_at_us: u64,
    last_exit: Option<ExitReason>,
}

impl ServiceSlot {
    const EMPTY: Self = Self {
        occupied: false,
        spec: ServiceSpec {
            id: ServiceId(0),
            name: ServiceName {
                bytes: [0; MAX_SERVICE_NAME_BYTES],
                len: 0,
            },
            kind: ServiceKind::System,
            image_id: 0,
            capability_profile: 0,
            restart: RestartPolicy::NEVER,
        },
        dependencies: [None; MAX_SERVICE_DEPENDENCIES],
        dependency_count: 0,
        state: ServiceState::Stopped,
        ready: false,
        process: None,
        generation: 0,
        restart_count: 0,
        window_started_us: 0,
        restart_at_us: 0,
        last_exit: None,
    };
}

/// Heap-free supervisor for isolated Ring 3 drivers and system services.
///
/// A crash only changes its owning slot. Other services keep their process,
/// capabilities, and generation.
pub struct Supervisor<const CAPACITY: usize = DEFAULT_SERVICE_CAPACITY> {
    services: [ServiceSlot; CAPACITY],
}

impl<const CAPACITY: usize> Supervisor<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            services: [ServiceSlot::EMPTY; CAPACITY],
        }
    }

    pub fn register(&mut self, spec: ServiceSpec) -> Result<(), SupervisorError> {
        self.register_with_dependencies(spec, &[])
    }

    pub fn register_with_dependencies(
        &mut self,
        spec: ServiceSpec,
        dependencies: &[ServiceId],
    ) -> Result<(), SupervisorError> {
        if spec.id.raw() == 0
            || spec.image_id == 0
            || spec.name.len == 0
            || self
                .services
                .iter()
                .any(|slot| slot.occupied && slot.spec.id == spec.id)
        {
            return Err(
                if self
                    .services
                    .iter()
                    .any(|slot| slot.occupied && slot.spec.id == spec.id)
                {
                    SupervisorError::AlreadyRegistered
                } else {
                    SupervisorError::InvalidService
                },
            );
        }
        validate_policy(spec.restart)?;
        if dependencies.len() > MAX_SERVICE_DEPENDENCIES {
            return Err(SupervisorError::Capacity);
        }
        for (index, dependency) in dependencies.iter().enumerate() {
            if *dependency == spec.id
                || dependencies[..index].contains(dependency)
                || self.slot_index(*dependency).is_err()
            {
                return Err(if *dependency == spec.id {
                    SupervisorError::DependencyCycle
                } else if self.slot_index(*dependency).is_err() {
                    SupervisorError::MissingDependency
                } else {
                    SupervisorError::InvalidService
                });
            }
        }
        let slot = self
            .services
            .iter_mut()
            .find(|slot| !slot.occupied)
            .ok_or(SupervisorError::Capacity)?;
        *slot = ServiceSlot {
            occupied: true,
            spec,
            dependency_count: dependencies.len(),
            ..ServiceSlot::EMPTY
        };
        for (index, dependency) in dependencies.iter().copied().enumerate() {
            slot.dependencies[index] = Some(dependency);
        }
        Ok(())
    }

    pub fn add_dependency(
        &mut self,
        service: ServiceId,
        dependency: ServiceId,
    ) -> Result<(), SupervisorError> {
        let service_index = self.slot_index(service)?;
        if self.slot_index(dependency).is_err() {
            return Err(SupervisorError::MissingDependency);
        }
        let slot = &mut self.services[service_index];
        if slot.dependencies[..slot.dependency_count].contains(&Some(dependency)) {
            return Err(SupervisorError::AlreadyRegistered);
        }
        if dependency == service {
            return Err(SupervisorError::DependencyCycle);
        }
        if slot.dependency_count == MAX_SERVICE_DEPENDENCIES {
            return Err(SupervisorError::Capacity);
        }
        slot.dependencies[slot.dependency_count] = Some(dependency);
        slot.dependency_count += 1;
        if self.topological_order().is_err() {
            let slot = &mut self.services[service_index];
            slot.dependency_count -= 1;
            slot.dependencies[slot.dependency_count] = None;
            return Err(SupervisorError::DependencyCycle);
        }
        Ok(())
    }

    pub fn start<R: SupervisorRuntime>(
        &mut self,
        id: ServiceId,
        runtime: &mut R,
    ) -> Result<SupervisorEvent, SupervisorError> {
        let mut no_interruption = NoInterruption;
        self.start_with_interruption(id, runtime, &mut no_interruption)
    }

    pub fn start_with_interruption<R: SupervisorRuntime, I: InterruptionInjector>(
        &mut self,
        id: ServiceId,
        runtime: &mut R,
        injector: &mut I,
    ) -> Result<SupervisorEvent, SupervisorError> {
        let index = self.slot_index(id)?;
        if self.services[index].state == ServiceState::Running {
            return Err(SupervisorError::AlreadyRegistered);
        }
        if !self.dependencies_ready(index) {
            return Err(SupervisorError::DependencyNotReady);
        }
        spawn(&mut self.services[index], runtime, injector)
    }

    pub fn report_exit<R: SupervisorRuntime>(
        &mut self,
        process: ProcessId,
        reason: ExitReason,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<SupervisorEvent, SupervisorError> {
        let index = self
            .services
            .iter()
            .position(|slot| slot.occupied && slot.process == Some(process))
            .ok_or(SupervisorError::StaleExit)?;
        let slot = &mut self.services[index];

        runtime
            .fence_process(process)
            .map_err(|_| SupervisorError::FenceFailed)?;
        slot.process = None;
        slot.ready = false;
        slot.last_exit = Some(reason);

        let ExitReason::Crash(crash) = reason else {
            slot.state = ServiceState::Stopped;
            return Ok(SupervisorEvent::Stopped {
                service: slot.spec.id,
            });
        };

        if slot.spec.restart.max_restarts == 0 {
            slot.state = ServiceState::Failed;
            return Ok(SupervisorEvent::Failed {
                service: slot.spec.id,
                reason: crash,
            });
        }

        if now_us.saturating_sub(slot.window_started_us) >= slot.spec.restart.window_us {
            slot.window_started_us = now_us;
            slot.restart_count = 0
        }
        if slot.restart_count >= slot.spec.restart.max_restarts {
            slot.state = ServiceState::Failed;
            return Ok(SupervisorEvent::Failed {
                service: slot.spec.id,
                reason: crash,
            });
        }

        let shift = core::cmp::min(slot.restart_count as u32, 63);
        let backoff = slot
            .spec
            .restart
            .initial_backoff_us
            .saturating_mul(1u64 << shift)
            .min(slot.spec.restart.max_backoff_us);
        slot.restart_count += 1;
        slot.restart_at_us = now_us.saturating_add(backoff);
        slot.state = ServiceState::Backoff;
        Ok(SupervisorEvent::RestartScheduled {
            service: slot.spec.id,
            at_us: slot.restart_at_us,
            reason: crash,
        })
    }

    /// Starts one service whose recovery delay has elapsed.
    pub fn tick<R: SupervisorRuntime>(
        &mut self,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<Option<SupervisorEvent>, SupervisorError> {
        let mut no_interruption = NoInterruption;
        self.tick_with_interruption(now_us, runtime, &mut no_interruption)
    }

    pub fn tick_with_interruption<R: SupervisorRuntime, I: InterruptionInjector>(
        &mut self,
        now_us: u64,
        runtime: &mut R,
        injector: &mut I,
    ) -> Result<Option<SupervisorEvent>, SupervisorError> {
        let Some(index) = (0..CAPACITY).find(|index| {
            let slot = &self.services[*index];
            slot.occupied
                && slot.state == ServiceState::Backoff
                && now_us >= slot.restart_at_us
                && self.dependencies_ready(*index)
        }) else {
            return Ok(None);
        };
        spawn(&mut self.services[index], runtime, injector).map(Some)
    }

    pub fn boot<R: SupervisorRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<LifecycleTrace, SupervisorError> {
        let mut trace = LifecycleTrace::new();
        trace.push(LifecycleEvent::BootStarted)?;
        self.boot_into(runtime, &mut trace)?;
        trace.push(LifecycleEvent::BootCompleted)?;
        Ok(trace)
    }

    pub fn shutdown<R: SupervisorRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<LifecycleTrace, SupervisorError> {
        let mut trace = LifecycleTrace::new();
        trace.push(LifecycleEvent::ShutdownStarted)?;
        self.shutdown_into(runtime, &mut trace)?;
        trace.push(LifecycleEvent::ShutdownCompleted)?;
        Ok(trace)
    }

    pub fn reboot<R: SupervisorRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<LifecycleTrace, SupervisorError> {
        let mut trace = LifecycleTrace::new();
        trace.push(LifecycleEvent::RebootStarted)?;
        trace.push(LifecycleEvent::ShutdownStarted)?;
        self.shutdown_into(runtime, &mut trace)?;
        trace.push(LifecycleEvent::BootStarted)?;
        self.boot_into(runtime, &mut trace)?;
        trace.push(LifecycleEvent::BootCompleted)?;
        trace.push(LifecycleEvent::RebootCompleted)?;
        Ok(trace)
    }

    pub fn readiness(&self, id: ServiceId) -> Result<ServiceReadiness, SupervisorError> {
        let index = self.slot_index(id)?;
        Ok(self.readiness_at(index, &mut [0; CAPACITY]))
    }

    pub fn accepts_work(&self, id: ServiceId) -> Result<(), SupervisorError> {
        if self.readiness(id)? == ServiceReadiness::Ready {
            Ok(())
        } else {
            Err(SupervisorError::NotReady)
        }
    }

    pub fn status(&self, id: ServiceId) -> Result<ServiceStatus, SupervisorError> {
        let index = self.slot_index(id)?;
        let slot = &self.services[index];
        Ok(ServiceStatus {
            spec: slot.spec,
            state: slot.state,
            readiness: self.readiness_at(index, &mut [0; CAPACITY]),
            process: slot.process,
            generation: slot.generation,
            restart_count: slot.restart_count,
            restart_at_us: slot.restart_at_us,
            last_exit: slot.last_exit,
        })
    }

    pub fn services(&self) -> impl Iterator<Item = ServiceStatus> + '_ {
        self.services
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.occupied)
            .map(|(index, slot)| {
                ServiceStatus {
                    spec: slot.spec,
                    state: slot.state,
                    readiness: self.readiness_at(index, &mut [0; CAPACITY]),
                    process: slot.process,
                    generation: slot.generation,
                    restart_count: slot.restart_count,
                    restart_at_us: slot.restart_at_us,
                    last_exit: slot.last_exit,
                }
            })
    }

    fn slot_index(&self, id: ServiceId) -> Result<usize, SupervisorError> {
        self.services
            .iter()
            .position(|slot| slot.occupied && slot.spec.id == id)
            .ok_or(SupervisorError::NotFound)
    }

    fn topological_order(&self) -> Result<([usize; CAPACITY], usize), SupervisorError> {
        let mut remaining = [false; CAPACITY];
        let mut count = 0;
        for (index, slot) in self.services.iter().enumerate() {
            if slot.occupied {
                remaining[index] = true;
                count += 1;
            }
        }

        let mut order = [0; CAPACITY];
        let mut length = 0;
        while length < count {
            let mut candidate: Option<usize> = None;
            for index in 0..CAPACITY {
                if !remaining[index] || !self.dependencies_satisfied_by_order(index, &remaining) {
                    continue;
                }
                if candidate.is_none_or(|current| {
                    self.services[index].spec.id.raw() < self.services[current].spec.id.raw()
                }) {
                    candidate = Some(index);
                }
            }
            let Some(index) = candidate else {
                return Err(SupervisorError::DependencyCycle);
            };
            remaining[index] = false;
            order[length] = index;
            length += 1;
        }
        Ok((order, length))
    }

    fn dependencies_satisfied_by_order(
        &self,
        index: usize,
        remaining: &[bool; CAPACITY],
    ) -> bool {
        self.services[index].dependencies[..self.services[index].dependency_count]
            .iter()
            .flatten()
            .all(|dependency| {
                self.slot_index(*dependency)
                    .ok()
                    .is_none_or(|dependency_index| !remaining[dependency_index])
            })
    }

    fn dependencies_ready(&self, index: usize) -> bool {
        self.services[index].dependencies[..self.services[index].dependency_count]
            .iter()
            .flatten()
            .all(|dependency| {
                self.slot_index(*dependency)
                    .ok()
                    .is_some_and(|dependency_index| {
                        self.readiness_at(dependency_index, &mut [0; CAPACITY])
                            == ServiceReadiness::Ready
                    })
            })
    }

    fn readiness_at(&self, index: usize, visiting: &mut [u8; CAPACITY]) -> ServiceReadiness {
        let slot = &self.services[index];
        if !slot.occupied || slot.state != ServiceState::Running || !slot.ready {
            return ServiceReadiness::Waiting;
        }
        if visiting[index] != 0 {
            return ServiceReadiness::Waiting;
        }
        visiting[index] = 1;
        let ready = slot.dependencies[..slot.dependency_count]
            .iter()
            .flatten()
            .all(|dependency| {
                self.slot_index(*dependency)
                    .ok()
                    .is_some_and(|dependency_index| {
                        self.readiness_at(dependency_index, visiting) == ServiceReadiness::Ready
                    })
            });
        visiting[index] = 0;
        if ready {
            ServiceReadiness::Ready
        } else {
            ServiceReadiness::Waiting
        }
    }

    fn boot_into<R: SupervisorRuntime>(
        &mut self,
        runtime: &mut R,
        trace: &mut LifecycleTrace,
    ) -> Result<(), SupervisorError> {
        let (order, length) = self.topological_order()?;
        for index in order[..length].iter().copied() {
            if self.services[index].state == ServiceState::Running {
                if self.readiness_at(index, &mut [0; CAPACITY]) != ServiceReadiness::Ready {
                    return Err(SupervisorError::DependencyNotReady);
                }
                continue;
            }
            if self.services[index].state != ServiceState::Stopped {
                return Err(SupervisorError::ServiceFailed);
            }
            if !self.dependencies_ready(index) {
                return Err(SupervisorError::DependencyNotReady);
            }
            let event = spawn(&mut self.services[index], runtime, &mut NoInterruption)?;
            if let SupervisorEvent::Started {
                service,
                generation,
                ..
            } = event
            {
                trace.push(LifecycleEvent::ServiceStarted { service, generation })?;
                trace.push(LifecycleEvent::ServiceReady { service })?;
            }
        }
        Ok(())
    }

    fn shutdown_into<R: SupervisorRuntime>(
        &mut self,
        runtime: &mut R,
        trace: &mut LifecycleTrace,
    ) -> Result<(), SupervisorError> {
        let (order, length) = self.topological_order()?;
        for index in order[..length].iter().rev().copied() {
            let process = self.services[index].process;
            if let Some(process) = process {
                runtime
                    .fence_process(process)
                    .map_err(|_| SupervisorError::FenceFailed)?;
            }
            let service = self.services[index].spec.id;
            let slot = &mut self.services[index];
            slot.process = None;
            slot.ready = false;
            slot.state = ServiceState::Stopped;
            slot.restart_count = 0;
            slot.window_started_us = 0;
            slot.restart_at_us = 0;
            trace.push(LifecycleEvent::ServiceStopped { service })?;
        }
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for Supervisor<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_policy(policy: RestartPolicy) -> Result<(), SupervisorError> {
    if policy == RestartPolicy::NEVER
        || (policy.max_restarts != 0
            && policy.window_us != 0
            && policy.initial_backoff_us != 0
            && policy.max_backoff_us >= policy.initial_backoff_us)
    {
        Ok(())
    } else {
        Err(SupervisorError::InvalidPolicy)
    }
}

fn spawn<R: SupervisorRuntime>(
    slot: &mut ServiceSlot,
    runtime: &mut R,
    injector: &mut impl InterruptionInjector,
) -> Result<SupervisorEvent, SupervisorError> {
    let generation = slot.generation.wrapping_add(1).max(1);
    let process = runtime
        .spawn(SpawnRequest {
            service: slot.spec.id,
            image_id: slot.spec.image_id,
            capability_profile: slot.spec.capability_profile,
            generation,
        })
        .map_err(|_| SupervisorError::SpawnFailed)?;
    slot.generation = generation;
    if injector.checkpoint(CrashDomain::CompilerJob, CrashBoundary::ServiceRestart) {
        return Err(SupervisorError::Interrupted)
    }
    slot.process = Some(process);
    slot.state = ServiceState::Running;
    slot.ready = true;
    Ok(SupervisorEvent::Started {
        service: slot.spec.id,
        process,
        generation,
    })
}
