#![no_std]
#![forbid(unsafe_code)]

mod manifest;
mod supervisor;

pub use manifest::{
    APP_MANIFEST_SCHEMA, AppManifest, ApplicationKind, BoundedText, CapabilityKind,
    CapabilityRequest, CapabilityRights, MAX_APP_CAPABILITIES, MAX_APP_NAME_BYTES,
    MAX_RESOURCE_NAME_BYTES, ManifestError, Placement, RestartMode, RuntimeSpec,
};
pub use supervisor::{
    AppSpawnRequest, ApplicationEvent, ApplicationId, ApplicationRuntime, ApplicationState,
    ApplicationStatus, ApplicationSupervisor, CapabilityPolicy, CapabilityRule, CapabilitySet,
    DEFAULT_APPLICATION_CAPACITY, ExecutableImage, PackageLaunchError, PolicyError,
    SupervisorError,
};
