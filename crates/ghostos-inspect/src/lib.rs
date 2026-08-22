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
mod runbook;

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
pub use ghostos_audit::ObsolescenceReport;
pub use ghostos_observability::{
    CacheEvent, CacheKind, CachePolicy, CachePolicyError, CachePolicyRegistry,
    CachePolicyReport, HealthError, HealthReport, HealthState, HealthTransport,
    OperationalHealth, SloDefinition, SloError, SloKind, SloMeasurement, SloObservation,
    SloReport, SloReportStatus, BUDGET_SCALE, CACHE_RATE_SCALE, DEFAULT_MAX_AGE_US,
    DEFAULT_OBJECTIVE_PER_MILLION, DEFAULT_WINDOW_US, MAX_CACHE_POLICIES,
    MAX_OPERATIONAL_HEALTH, SLO_COUNT,
};
pub use ghostos_admission::{
    AdmissionAction, AdmissionController, AdmissionError, AdmissionLease, AdmissionOutcome,
    AdmissionPolicy, AdmissionPriority, AdmissionReason, AdmissionReport, AdmissionStats,
    WorkClass,
};
pub use service::{
    InspectError, InspectionProvider, InspectionService, TelemetryStore, View,
};
pub use shell::ShellInspectionSource;
pub use storage::{
    CapacitySample, DeviceHealth, StorageDeviceSample, StorageKind, StorageReport,
    SynFsVolumeSample, MAX_CAPACITY_SAMPLES, MAX_STORAGE_DEVICES, MAX_GHOSTFS_VOLUMES,
};
pub use text::Name;
pub use runbook::{
    CompatibilityMetadata, DependencyMetadata, RecoveryMetadata, Runbook, RunbookError,
    RunbookGenerator, RunbookLink, RunbookMetadata, RunbookSource, QuotaMetadata,
    ServiceHealthMetadata, DEFAULT_RUNBOOK_CAPACITY,
};
