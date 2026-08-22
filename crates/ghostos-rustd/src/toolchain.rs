use ghostos_init::ProcessId;
use ghostos_pkg::PackageDaemon;
use ghostos_system_model::ContentId;

use crate::{BuildRequest, CompilerCapabilities, Error, Target, Text, MAX_PATH_BYTES};

pub const MAX_TOOLCHAIN_COMPONENTS: usize = 8;
pub const MAX_TOOLCHAIN_ASSETS: usize = 8;
pub const MAX_TOOLCHAIN_STEPS: usize = MAX_TOOLCHAIN_COMPONENTS;
pub const MAX_DYNAMIC_ARTIFACTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind {
    Cargo,
    Rustc,
    Rustdoc,
    Linker,
    BuildScript,
    ProcMacro,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolchainAssetKind {
    Codegen,
    Sysroot,
    TargetLibraries,
    Sources,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainAsset {
    pub kind: ToolchainAssetKind,
    pub package: ContentId,
    pub content: ContentId,
    pub path: Text<MAX_PATH_BYTES>,
    pub target: Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicArtifactKind {
    BuildScript,
    ProcMacro,
    CodeGenerator,
    TestBinary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DynamicArtifact {
    pub kind: DynamicArtifactKind,
    pub package: ContentId,
    pub payload: ContentId,
    pub executable: Text<MAX_PATH_BYTES>,
    pub target: Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactError {
    Capacity,
    InvalidArtifact,
    PackageNotAuthorized,
    NotFound,
}

/// Build-local registry for dynamic build-script and proc-macro images.
/// Entries are accepted only after package verification and are revoked when
/// the build workspace is released.
pub struct ArtifactSandbox {
    artifacts: [Option<DynamicArtifact>; MAX_DYNAMIC_ARTIFACTS],
    owners: [Option<crate::JobId>; MAX_DYNAMIC_ARTIFACTS],
}

impl ArtifactSandbox {
    pub const fn new() -> Self {
        Self {
            artifacts: [None; MAX_DYNAMIC_ARTIFACTS],
            owners: [None; MAX_DYNAMIC_ARTIFACTS],
        }
    }

    pub fn stage<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        self.stage_owned(None, packages, artifact)
    }

    pub fn stage_for_job<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        job: crate::JobId,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        self.stage_owned(Some(job), packages, artifact)
    }

    fn stage_owned<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        owner: Option<crate::JobId>,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        artifact: DynamicArtifact,
    ) -> Result<(), ArtifactError> {
        if artifact.package.is_zero() || artifact.payload.is_zero() {
            return Err(ArtifactError::InvalidArtifact);
        }
        packages
            .authorize_instantiation(artifact.package)
            .map_err(|_| ArtifactError::PackageNotAuthorized)?;
        if self.artifacts.iter().flatten().any(|current| current.payload == artifact.payload) {
            return Err(ArtifactError::InvalidArtifact);
        }
        let index = self
            .artifacts
            .iter()
            .position(Option::is_none)
            .ok_or(ArtifactError::Capacity)?;
        self.artifacts[index] = Some(artifact);
        self.owners[index] = owner;
        Ok(())
    }

    pub fn resolve(&self, payload: ContentId) -> Result<DynamicArtifact, ArtifactError> {
        self.artifacts
            .iter()
            .flatten()
            .find(|artifact| artifact.payload == payload)
            .copied()
            .ok_or(ArtifactError::NotFound)
    }

    pub fn release(&mut self, payload: ContentId) -> Result<(), ArtifactError> {
        let index = self
            .artifacts
            .iter()
            .position(|artifact| artifact.is_some_and(|artifact| artifact.payload == payload))
            .ok_or(ArtifactError::NotFound)?;
        self.artifacts[index] = None;
        self.owners[index] = None;
        Ok(())
    }

    pub fn release_job(&mut self, job: crate::JobId) {
        for index in 0..MAX_DYNAMIC_ARTIFACTS {
            if self.owners[index] == Some(job) {
                self.artifacts[index] = None;
                self.owners[index] = None;
            }
        }
    }

    pub fn release_all(&mut self) {
        self.artifacts.fill(None);
        self.owners.fill(None);
    }

    pub fn active(&self) -> impl Iterator<Item = DynamicArtifact> + '_ {
        self.artifacts.iter().flatten().copied()
    }
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
    MissingAsset(ToolchainAssetKind),
    CapabilityDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainPolicy {
    pub allow_build_scripts: bool,
    pub allow_proc_macros: bool,
    pub allow_network: bool,
    pub max_steps: u8,
    pub capabilities: CompilerCapabilities,
}

impl ToolchainPolicy {
    pub const OFFLINE: Self = Self {
        allow_build_scripts: true,
        allow_proc_macros: true,
        allow_network: false,
        max_steps: MAX_TOOLCHAIN_STEPS as u8,
        capabilities: CompilerCapabilities::MINIMUM,
    };

    pub const RESTRICTED: Self = Self {
        allow_build_scripts: false,
        allow_proc_macros: false,
        allow_network: false,
        max_steps: 3,
        capabilities: CompilerCapabilities::MINIMUM,
    };
}

#[derive(Clone, Copy)]
pub struct ToolchainManifest {
    components: [Option<ToolchainComponent>; MAX_TOOLCHAIN_COMPONENTS],
    assets: [Option<ToolchainAsset>; MAX_TOOLCHAIN_ASSETS],
}

impl ToolchainManifest {
    pub const fn new() -> Self {
        Self {
            components: [None; MAX_TOOLCHAIN_COMPONENTS],
            assets: [None; MAX_TOOLCHAIN_ASSETS],
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

    pub fn install_asset_authorized<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        asset: ToolchainAsset,
    ) -> Result<(), ToolchainError> {
        if asset.package.is_zero() || asset.content.is_zero() {
            return Err(ToolchainError::InvalidComponent);
        }
        packages
            .authorize_instantiation(asset.package)
            .map_err(|_| ToolchainError::PackageNotAuthorized)?;
        if self.assets.iter().flatten().any(|current| current.kind == asset.kind) {
            return Err(ToolchainError::DuplicateComponent);
        }
        let slot = self
            .assets
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ToolchainError::Capacity)?;
        *slot = Some(asset);
        Ok(())
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

    pub fn assets(&self) -> impl Iterator<Item = ToolchainAsset> + '_ {
        self.assets.iter().flatten().copied()
    }

    pub fn asset(&self, kind: ToolchainAssetKind) -> Option<ToolchainAsset> {
        self.assets.iter().flatten().find(|asset| asset.kind == kind).copied()
    }

    pub fn validate_complete(&self, target: Target) -> Result<(), ToolchainError> {
        for kind in [ToolKind::Cargo, ToolKind::Rustc, ToolKind::Rustdoc, ToolKind::Linker] {
            let component = self.require(kind)?;
            if component.target != target {
                return Err(ToolchainError::InvalidComponent);
            }
        }
        for kind in [
            ToolchainAssetKind::Sysroot,
            ToolchainAssetKind::TargetLibraries,
            ToolchainAssetKind::Sources,
        ] {
            let asset = self.asset(kind).ok_or(ToolchainError::MissingAsset(kind))?;
            if asset.target != target {
                return Err(ToolchainError::InvalidComponent);
            }
        }
        Ok(())
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
    pub capabilities: CompilerCapabilities,
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
        if !policy.capabilities.contains(request.capabilities) {
            return Err(ToolchainError::CapabilityDenied);
        }
        if request.build.network == crate::NetworkPolicy::Allowed
            && !request.capabilities.contains(CompilerCapabilities::NETWORK)
        {
            return Err(ToolchainError::CapabilityDenied);
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
