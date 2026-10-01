use ghostos_boot_protocol::{BootInfo, MemoryKind};
use ghostos_power::{
    AcpiError, AcpiMemory, AcpiPlatform, AddressSpace, FixedEvent, GenericAddress,
    PowerController, PowerIo, PowerState,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct CMemoryRegion {
    start: u64,
    length: u64,
    kind: u32,
    attributes: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CPowerRegister {
    address_space: u8,
    bit_width: u8,
    bit_offset: u8,
    access_size: u8,
    address: u64,
}

unsafe extern "C" {
    fn ghostos_power_acpi_read(
        regions: *const CMemoryRegion,
        region_count: usize,
        physical_offset: u64,
        physical_address: u64,
        destination: *mut u8,
        length: usize,
    ) -> bool;
    #[cfg(test)]
    fn ghostos_power_register_access_bytes(register: CPowerRegister, bytes: *mut u8) -> i32;
    #[cfg(test)]
    fn ghostos_power_extract_field(raw: u64, register: CPowerRegister) -> u64;
    fn ghostos_power_register_read(register: CPowerRegister, value: *mut u64) -> i32;
    fn ghostos_power_register_write(register: CPowerRegister, value: u64) -> i32;
    fn ghostos_power_vm_shutdown();
    fn ghostos_power_vm_reboot();
    fn ghostos_power_reboot_fallback();
}

struct PhysicalAcpiMemory<'a> {
    boot_info: &'a BootInfo,
}

impl AcpiMemory for PhysicalAcpiMemory<'_> {
    fn read(
        &self,
        physical_address: u64,
        destination: &mut [u8],
    ) -> Result<(), AcpiError> {
        let regions = self.boot_info.regions();
        let c_regions: [CMemoryRegion; ghostos_boot_protocol::MAX_MEMORY_REGIONS] =
            core::array::from_fn(|index| {
                regions.get(index).map_or(
                    CMemoryRegion { start: 0, length: 0, kind: MemoryKind::Usable as u32, attributes: 0 },
                    |region| CMemoryRegion {
                        start: region.start,
                        length: region.length,
                        kind: region.kind as u32,
                        attributes: region.attributes,
                    },
                )
            });
        // SAFETY: C reads the copied region map and writes only the destination slice.
        if unsafe {
            ghostos_power_acpi_read(
                c_regions.as_ptr(),
                regions.len(),
                self.boot_info.physical_address_offset,
                physical_address,
                destination.as_mut_ptr(),
                destination.len(),
            )
        } {
            Ok(())
        } else {
            Err(AcpiError::InvalidAddress)
        }
    }
}

pub fn discover(boot_info: &'static BootInfo) -> Option<AcpiPlatform> {
    AcpiPlatform::discover(&PhysicalAcpiMemory { boot_info }, boot_info.rsdp_address).ok()
}

pub fn enable(platform: &AcpiPlatform) -> Result<(), AcpiError> {
    let mut controller = PowerController::new(*platform, PlatformIo);
    controller.enable_acpi(1_000_000)?;
    if platform.fixed.pm1a_event.is_some() {
        controller.set_fixed_event_enabled(FixedEvent::PowerButton, true)
    } else {
        Ok(())
    }
}

pub fn power_button_pressed(platform: &AcpiPlatform) -> bool {
    PowerController::new(*platform, PlatformIo)
        .poll_fixed_events()
        .is_ok_and(|events| events.contains(FixedEvent::PowerButton))
}

pub fn battery_report(
    boot_info: &'static BootInfo,
    platform: Option<&AcpiPlatform>,
) -> ghostos_power::BatteryReport {
    let Some(platform) = platform else {
        return ghostos_power::BatteryReport::UNKNOWN
    };
    platform.battery_report(&PhysicalAcpiMemory { boot_info })
}

pub fn suspend(platform: Option<&AcpiPlatform>) -> Result<(), AcpiError> {
    let platform = platform.ok_or(AcpiError::Unsupported)?;
    let mut controller = PowerController::new(*platform, PlatformIo);
    controller.enable_acpi(1_000_000)?;
    if !platform.fixed.reduced_hardware && platform.fixed.pm1a_event.is_some() {
        let _ = controller.poll_fixed_events()?;
        for event in [
            FixedEvent::PowerButton,
            FixedEvent::SleepButton,
            FixedEvent::PcieWake,
        ] {
            controller.set_fixed_event_enabled(event, true)?
        }
    }

    crate::println!("Suspending GhostOS...");
    crate::arch::disable_interrupts();
    let requested = controller.request(PowerState::Suspend);
    let result = match requested {
        Ok(()) => resume(platform),
        Err(error) => Err(error),
    };
    crate::arch::enable_interrupts();
    if result.is_ok() {
        crate::println!("GhostOS resumed")
    }
    result
}

pub fn resume(platform: &AcpiPlatform) -> Result<(), AcpiError> {
    let mut controller = PowerController::new(*platform, PlatformIo);
    controller.enable_acpi(1_000_000)?;
    if platform.fixed.pm1a_event.is_some() {
        let _ = controller.poll_fixed_events()?;
    }
    Ok(())
}

pub fn shutdown(platform: Option<&AcpiPlatform>) -> ! {
    crate::println!("Shutting down GhostOS...");
    let acpi_requested = if let Some(platform) = platform {
        let mut controller = PowerController::new(*platform, PlatformIo);
        let _ = controller.enable_acpi(1_000_000);
        controller.request(PowerState::SoftOff).is_ok()
    } else {
        false
    };
    if !acpi_requested {
        request_vm_shutdown()
    }
    crate::halt()
}

pub fn reboot(platform: Option<&AcpiPlatform>) -> ! {
    crate::println!("Rebooting GhostOS...");
    let acpi_requested = if let Some(platform) = platform {
        let mut controller = PowerController::new(*platform, PlatformIo);
        let _ = controller.enable_acpi(1_000_000);
        controller.request(PowerState::Reboot).is_ok()
    } else {
        false
    };
    if !acpi_requested {
        request_vm_reboot()
    }

    unsafe {
        ghostos_power_reboot_fallback()
    }
    crate::halt()
}

struct PlatformIo;

fn request_vm_reboot() {
    unsafe {
        ghostos_power_vm_reboot()
    }
}

fn request_vm_shutdown() {
    unsafe {
        ghostos_power_vm_shutdown()
    }
}

impl PowerIo for PlatformIo {
    fn read(&mut self, register: GenericAddress) -> Result<u64, AcpiError> {
        let mut value = 0;
        // SAFETY: C performs validated volatile access for the ACPI register.
        map_io_error(unsafe { ghostos_power_register_read(c_register(register), &mut value) })?;
        Ok(value)
    }

    fn write(
        &mut self,
        register: GenericAddress,
        value: u64,
    ) -> Result<(), AcpiError> {
        // SAFETY: C validates the address and performs the selected volatile write.
        map_io_error(unsafe { ghostos_power_register_write(c_register(register), value) })
    }
}

fn c_register(register: GenericAddress) -> CPowerRegister {
    let address_space = match register.address_space {
        AddressSpace::SystemMemory => 0,
        AddressSpace::SystemIo => 1,
        _ => 0xff,
    };
    CPowerRegister {
        address_space,
        bit_width: register.bit_width,
        bit_offset: register.bit_offset,
        access_size: register.access_size,
        address: register.address,
    }
}

fn map_io_error(error: i32) -> Result<(), AcpiError> {
    match error {
        0 => Ok(()),
        1 => Err(AcpiError::InvalidAddress),
        _ => Err(AcpiError::Unsupported),
    }
}

#[cfg(test)]
fn access_bytes(register: GenericAddress) -> Result<u8, AcpiError> {
    let mut bytes = 0;
    // SAFETY: C helper only validates the register description and writes one byte.
    map_io_error(unsafe { ghostos_power_register_access_bytes(c_register(register), &mut bytes) })?;
    Ok(bytes)
}

#[cfg(test)]
fn extract_field(raw: u64, register: GenericAddress) -> u64 {
    // SAFETY: C helper extracts the validated ACPI field without accessing hardware.
    unsafe { ghostos_power_extract_field(raw, c_register(register)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(bytes: u8) -> GenericAddress {
        GenericAddress {
            address_space: AddressSpace::SystemMemory,
            bit_width: bytes * 8,
            bit_offset: 0,
            access_size: bytes,
            address: 0x1000,
        }
    }

    #[test]
    fn acpi_register_access_rejects_zero_misaligned_and_unsupported_addresses() {
        assert_eq!(access_bytes(register(1)), Ok(1));
        assert_eq!(
            access_bytes(GenericAddress { address: 0, ..register(1) }),
            Err(AcpiError::InvalidAddress)
        );
        assert_eq!(
            access_bytes(GenericAddress { address: 0x1001, ..register(2) }),
            Err(AcpiError::InvalidAddress)
        );
        assert_eq!(
            access_bytes(GenericAddress { access_size: 5, ..register(1) }),
            Err(AcpiError::Unsupported)
        );
    }

    #[test]
    fn acpi_bit_fields_are_extracted_without_overflow() {
        let field = GenericAddress {
            bit_offset: 4,
            bit_width: 8,
            ..register(2)
        };
        assert_eq!(extract_field(0xabcd, field), 0xbc);
        let wide = GenericAddress {
            bit_offset: 0,
            bit_width: 64,
            ..register(8)
        };
        assert_eq!(extract_field(u64::MAX, wide), u64::MAX);
    }
}
