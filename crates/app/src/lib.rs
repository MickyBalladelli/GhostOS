#![no_std]
#![forbid(unsafe_code)]

mod manifest;
mod loader;
mod process;
mod supervisor;

pub use manifest::{
    APP_MANIFEST_SCHEMA, AppManifest, ApplicationKind, BoundedText, CapabilityKind,
    AppTarget, ApplicationDependency, ApplicationResources, CapabilityRequest, CapabilityRights,
    MAX_APP_CAPABILITIES, MAX_APP_DEPENDENCIES, MAX_APP_NAME_BYTES, MAX_APPLICATION_CPU_TIME_US,
    MAX_APPLICATION_HEAP_BYTES, MAX_APPLICATION_MEMORY_BYTES, MAX_RESOURCE_NAME_BYTES,
    ManifestError, Placement, RestartMode, RuntimeSpec,
};
pub use loader::{
    load_image, measure_executable_pages, parse_image, ImageArchitecture, ImageFormat, ImageLayout,
    ImageLoadRequest, ImageMapper, LoadSegment, LoadedImage, LoaderError, Mapping, MappingRequest,
    PageMeasurer, ProcessArguments, ProcessContext, RelativeRelocation, RuntimeSegment,
    SegmentPermissions, StackRequest, TlsImage, TlsRequest, DEFAULT_GUARD_PAGES,
    DEFAULT_HEAP_BYTES, DEFAULT_STACK_BYTES, MAX_ARGUMENTS, MAX_ENVIRONMENT, MAX_EXECUTABLE_PAGES,
    MAX_LOAD_SEGMENTS, MAX_RELOCATIONS, MAX_STACK_ARGUMENT_BYTES, PAGE_SIZE,
};
pub use process::{
    NativeExecRequest, NativeSpawnRequest, ProcessBackend, ProcessError, ProcessExit,
    ProcessLimits, ProcessState, ProcessStatus, ProcessSupervisor, ProcessUsage,
    DEFAULT_CANCEL_GRACE_US, DEFAULT_PROCESS_CAPACITY,
};
pub use supervisor::{
    AppSpawnRequest, ApplicationEvent, ApplicationId, ApplicationRuntime, ApplicationState,
    ApplicationStatus, ApplicationSupervisor, CapabilityPolicy, CapabilityRule, CapabilitySet,
    DEFAULT_APPLICATION_CAPACITY, ExecutableImage, PackageLaunchError, PolicyError,
    SupervisorError,
};
