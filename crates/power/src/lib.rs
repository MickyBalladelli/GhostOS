#![no_std]
#![forbid(unsafe_code)]

mod acpi;
mod hotplug;
mod thermal;

pub use acpi::{
    AcpiError, AcpiMemory, AcpiPlatform, AddressSpace, FixedEvent, FixedEvents,
    FixedHardware, GenericAddress, PowerController, PowerIo, PowerState, ResetRegister,
    SleepTypes,
};
pub use hotplug::{
    HotPlugDevice, HotPlugError, HotPlugState, HotPlugTransition,
    begin_cxl_memory_removal, begin_nvme_storage_removal,
    complete_cxl_memory_removal, complete_nvme_storage_removal,
    handle_cxl_insertion, handle_nvme_insertion,
};
pub use thermal::{
    ThermalAction, ThermalActuator, ThermalManager, ThermalReading, ThermalSensor,
    ThermalSupervisor, ThermalTripPoints,
};
