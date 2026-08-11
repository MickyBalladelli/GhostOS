#![no_std]
#![forbid(unsafe_code)]

//! Capability-scoped snapshots used by `SHOW MEMORY`, `SHOW DISK`,
//! `SHOW CPU`, `SHOW USERS`, `SHOW PROCESS`, and `SHOW OBSOLETE`.

mod access;
mod activity;
mod cpu;
mod memory;
mod service;
mod shell;
mod storage;
mod text;

pub use access::{
    CAP_AUDIT_WORLD, InspectCapability, InspectionAuthority, InspectionRights,
    PrincipalId,
};
pub use activity::{
    ActivityRegistry, ActivityReport, ProcessId, ProcessSample, ProcessState,
    SessionId, SessionSample, UserSample, MAX_PROCESSES, MAX_SESSIONS,
    MAX_USERS,
};
pub use cpu::{CpuAccountant, CpuNodeSample, CpuReport, MAX_CPU_NODES};
pub use memory::{
    CxlLeaseSample, DsmPageSample, MemoryNodeSample, MemoryReport,
    MAX_CXL_LEASES, MAX_DSM_ALLOCATIONS, MAX_MEMORY_NODES,
};
pub use synos_audit::ObsolescenceReport;
pub use synos_observability::{
    CacheEvent, CacheKind, CachePolicy, CachePolicyError, CachePolicyRegistry,
    CachePolicyReport, HealthError, HealthReport, HealthState, HealthTransport,
    OperationalHealth, CACHE_RATE_SCALE, MAX_CACHE_POLICIES, MAX_OPERATIONAL_HEALTH,
};
pub use service::{
    InspectError, InspectionProvider, InspectionService, TelemetryStore, View,
};
pub use shell::ShellInspectionSource;
pub use storage::{
    CapacitySample, DeviceHealth, StorageDeviceSample, StorageKind, StorageReport,
    SynFsVolumeSample, MAX_CAPACITY_SAMPLES, MAX_STORAGE_DEVICES, MAX_SYNFS_VOLUMES,
};
pub use text::Name;
