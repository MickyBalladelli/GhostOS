#![no_std]
#![forbid(unsafe_code)]

mod acpi;
mod hotplug;
mod policy;
mod thermal;

pub use acpi::{
    AcpiError, AcpiMemory, AcpiPlatform, AddressSpace, FixedEvent, FixedEvents,
    BatteryReport, BatteryState, BatteryStatus, FixedHardware, GenericAddress,
    PowerController, PowerIo, PowerSource, PowerState, ResetRegister, SleepTypes,
};
pub use hotplug::{
    HotPlugDevice, HotPlugError, HotPlugState, HotPlugTransition,
    begin_cxl_memory_removal, begin_nvme_storage_removal,
    complete_cxl_memory_removal, complete_nvme_storage_removal,
    handle_cxl_insertion, handle_nvme_insertion,
};
pub use thermal::{
    ThermalAction, ThermalActuator, ThermalEvent, ThermalEventKind, ThermalEventLog,
    ThermalManager, ThermalReading, ThermalSensor, ThermalSupervisor, ThermalTripPoints,
    MAX_THERMAL_EVENTS,
};
pub use policy::{
    CpuIdleState, DevicePowerConfig, DevicePowerState, FrequencyDecision, IdleRequest,
    PlacementDecision, PowerBudget, PowerClusterConfig, PowerMetrics, PowerPolicy,
    PowerPolicyError, PowerPolicyIo, ProcessorSet, ThermalRecoveryMetrics, WorkloadClass,
    WorkloadRequest, MAX_FREQUENCY_POINTS, MAX_POWER_CLUSTERS, MAX_POWER_CPUS,
    MAX_POWER_DEVICES,
};
