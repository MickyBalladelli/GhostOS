use synos_init::ProcessId;
use synos_pkg::PackageDaemon;
use synos_system_model::ContentId;

use crate::{BuildRequest, Error, Target, Text, MAX_PATH_BYTES};

pub const MAX_TOOLCHAIN_COMPONENTS: usize = 5;
pub const MAX_TOOLCHAIN_STEPS: usize = MAX_TOOLCHAIN_COMPONENTS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind {
    Cargo,
    Rustc,
    Linker,
    BuildScript,
    ProcMacro,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainComponent {
    pub kind: ToolKind,
    pub package: ContentId,
    pub executable: Text<MAX_PATH_BYTES>,
    pub target: Target,
}

impl ToolchainComponent {
    pub fn new(
        kind: ToolKind,
        package: ContentId,
        executable: &str,
        target: Target,
    ) -> Result<Self, ToolchainError> {
        if package.is_zero() {
            return Err(ToolchainError::InvalidComponent);
        }
        Ok(Self {
            kind,
            package,
            executable: Text::new(executable).map_err(|_| ToolchainError::InvalidComponent)?,
            target,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolchainError {
    Capacity,
    DuplicateComponent,
    InvalidComponent,
    MissingComponent(ToolKind),
    PackageNotAuthorized,
    InvalidRequest,
    BuildScriptsDenied,
    ProcMacrosDenied,
    NetworkDenied,
    StepCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainPolicy {
    pub allow_build_scripts: bool,
    pub allow_proc_macros: bool,
    pub allow_network: bool,
    pub max_steps: u8,
}

impl ToolchainPolicy {
    pub const OFFLINE: Self = Self {
        allow_build_scripts: true,
        allow_proc_macros: true,
        allow_network: false,
        max_steps: MAX_TOOLCHAIN_STEPS as u8,
    };

    pub const RESTRICTED: Self = Self {
        allow_build_scripts: false,
        allow_proc_macros: false,
        allow_network: false,
        max_steps: 3,
    };
}

#[derive(Clone, Copy)]
pub struct ToolchainManifest {
    components: [Option<ToolchainComponent>; MAX_TOOLCHAIN_COMPONENTS],
}

impl ToolchainManifest {
    pub const fn new() -> Self {
        Self {
            components: [None; MAX_TOOLCHAIN_COMPONENTS],
        }
    }

    fn install(&mut self, component: ToolchainComponent) -> Result<(), ToolchainError> {
        if self
            .components
            .iter()
            .flatten()
            .any(|current| current.kind == component.kind)
        {
            return Err(ToolchainError::DuplicateComponent);
        }
        let slot = self
            .components
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ToolchainError::Capacity)?;
        *slot = Some(component);
        Ok(())
    }

    pub fn install_authorized<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        component: ToolchainComponent,
    ) -> Result<(), ToolchainError> {
        packages
            .authorize_instantiation(component.package)
            .map_err(|_| ToolchainError::PackageNotAuthorized)?;
        self.install(component)
    }

    pub fn component(&self, kind: ToolKind) -> Option<ToolchainComponent> {
        self.components
            .iter()
            .flatten()
            .find(|component| component.kind == kind)
            .copied()
    }

    pub fn components(&self) -> impl Iterator<Item = ToolchainComponent> + '_ {
        self.components.iter().flatten().copied()
    }

    fn require(&self, kind: ToolKind) -> Result<ToolchainComponent, ToolchainError> {
        self.component(kind)
            .ok_or(ToolchainError::MissingComponent(kind))
    }
}

impl Default for ToolchainManifest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainRequest {
    pub build: BuildRequest,
    pub run_build_scripts: bool,
    pub run_proc_macros: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainStep {
    pub ordinal: u8,
    pub kind: ToolKind,
    pub package: ContentId,
    pub executable: Text<MAX_PATH_BYTES>,
    pub target: Target,
}

#[derive(Clone, Copy)]
pub struct ToolchainPlan {
    request: ToolchainRequest,
    steps: [Option<ToolchainStep>; MAX_TOOLCHAIN_STEPS],
    length: u8,
}

impl ToolchainPlan {
    pub fn build(
        manifest: &ToolchainManifest,
        request: ToolchainRequest,
        policy: ToolchainPolicy,
    ) -> Result<Self, ToolchainError> {
        request
            .build
            .validate()
            .map_err(|_| ToolchainError::InvalidRequest)?;
        if request.build.network == crate::NetworkPolicy::Allowed && !policy.allow_network {
            return Err(ToolchainError::NetworkDenied);
        }
        if !request.build.locked && !policy.allow_network {
            return Err(ToolchainError::InvalidRequest);
        }
        if request.run_build_scripts && !policy.allow_build_scripts {
            return Err(ToolchainError::BuildScriptsDenied);
        }
        if request.run_proc_macros && !policy.allow_proc_macros {
            return Err(ToolchainError::ProcMacrosDenied);
        }

        let mut plan = Self {
            request,
            steps: [None; MAX_TOOLCHAIN_STEPS],
            length: 0,
        };
        plan.push(manifest.require(ToolKind::Cargo)?)?;
        if request.run_build_scripts {
            plan.push(manifest.require(ToolKind::BuildScript)?)?;
        }
        if request.run_proc_macros {
            plan.push(manifest.require(ToolKind::ProcMacro)?)?;
        }
        plan.push(manifest.require(ToolKind::Rustc)?)?;
        plan.push(manifest.require(ToolKind::Linker)?)?;
        if plan.length > policy.max_steps || plan.length > MAX_TOOLCHAIN_STEPS as u8 {
            return Err(ToolchainError::StepCapacity);
        }
        Ok(plan)
    }

    pub const fn request(&self) -> ToolchainRequest {
        self.request
    }

    pub fn steps(&self) -> impl Iterator<Item = ToolchainStep> + '_ {
        self.steps.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.length as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    fn push(&mut self, component: ToolchainComponent) -> Result<(), ToolchainError> {
        if component.target != self.request.build.target {
            return Err(ToolchainError::InvalidComponent);
        }
        let slot = self
            .steps
            .get_mut(self.length as usize)
            .ok_or(ToolchainError::StepCapacity)?;
        *slot = Some(ToolchainStep {
            ordinal: self.length,
            kind: component.kind,
            package: component.package,
            executable: component.executable,
            target: component.target,
        });
        self.length += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolSpawnRequest {
    pub step: ToolchainStep,
    pub build: BuildRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolExit {
    pub code: i32,
}

pub trait ToolchainRuntime {
    type Error;

    fn spawn_tool(&mut self, request: ToolSpawnRequest) -> Result<ProcessId, Self::Error>;
    fn wait_tool(&mut self, process: ProcessId) -> Result<ToolExit, Self::Error>;
    fn fence_tool(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainReceipt {
    pub completed_steps: u8,
    pub final_status: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolchainExecutionError<E> {
    Runtime(E),
    ToolFailed { kind: ToolKind, code: i32 },
}

pub struct ToolchainExecutor;

impl ToolchainExecutor {
    pub const fn new() -> Self {
        Self
    }

    pub fn execute<R: ToolchainRuntime>(
        &self,
        plan: &ToolchainPlan,
        runtime: &mut R,
    ) -> Result<ToolchainReceipt, ToolchainExecutionError<R::Error>> {
        let mut completed_steps = 0;
        let mut final_status = 0;
        for step in plan.steps() {
            let process = runtime
                .spawn_tool(ToolSpawnRequest {
                    step,
                    build: plan.request().build,
                })
                .map_err(ToolchainExecutionError::Runtime)?;
            let exit = match runtime.wait_tool(process) {
                Ok(exit) => exit,
                Err(error) => {
                    let _ = runtime.fence_tool(process);
                    return Err(ToolchainExecutionError::Runtime(error));
                }
            };
            final_status = exit.code;
            if exit.code != 0 {
                let _ = runtime.fence_tool(process);
                return Err(ToolchainExecutionError::ToolFailed {
                    kind: step.kind,
                    code: exit.code,
                });
            }
            completed_steps += 1;
        }
        Ok(ToolchainReceipt {
            completed_steps,
            final_status,
        })
    }
}

impl Default for ToolchainExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Error> for ToolchainError {
    fn from(_: Error) -> Self {
        Self::InvalidRequest
    }
}
