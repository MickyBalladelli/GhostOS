use ghostos_boot_protocol::{BootInfo, MemoryKind};
use ghostos_power::{
    AcpiError, AcpiMemory, AcpiPlatform, AddressSpace, GenericAddress,
    ResetRegister, SleepTypes,
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

#[repr(C)]
#[derive(Clone, Copy)]
struct COptionalRegister {
    present: bool,
    register_value: CPowerRegister,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CPowerPlatform {
    pm1a_event: COptionalRegister,
    pm1b_event: COptionalRegister,
    pm1a_control: COptionalRegister,
    pm1b_control: COptionalRegister,
    pm1_event_bytes: u8,
    smi_command_port: u32,
    acpi_enable_value: u8,
    reset_present: bool,
    reset_register: CPowerRegister,
    reset_value: u8,
    reduced_hardware: bool,
    sleep_control: COptionalRegister,
    sleep_status: COptionalRegister,
    suspend_present: bool,
    soft_off_present: bool,
    suspend_a: u8,
    suspend_b: u8,
    soft_off_a: u8,
    soft_off_b: u8,
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
    fn ghostos_power_enable_acpi(platform: *const CPowerPlatform, spin_limit: usize) -> i32;
    fn ghostos_power_poll_events(platform: *const CPowerPlatform, events: *mut u16) -> i32;
    fn ghostos_power_set_event(platform: *const CPowerPlatform, event: u8, enabled: bool) -> i32;
    fn ghostos_power_request(platform: *const CPowerPlatform, state: u8) -> i32;
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
    enable_acpi(platform)?;
    if platform.fixed.pm1a_event.is_some() {
        set_event(platform, 1, true)
    } else {
        Ok(())
    }
}

pub fn power_button_pressed(platform: &AcpiPlatform) -> bool {
    poll_events(platform).is_ok_and(|events| events & (1 << 8) != 0)
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

fn c_optional(register: Option<GenericAddress>) -> COptionalRegister {
    COptionalRegister {
        present: register.is_some(),
        register_value: register.map_or(c_register(GenericAddress {
            address_space: AddressSpace::Other(0xff),
            bit_width: 0,
            bit_offset: 0,
            access_size: 0,
            address: 0,
        }), c_register),
    }
}

fn c_sleep(sleep: Option<SleepTypes>) -> (bool, u8, u8) {
    sleep.map_or((false, 0, 0), |sleep| (true, sleep.type_a, sleep.type_b))
}

fn c_platform(platform: &AcpiPlatform) -> CPowerPlatform {
    let fixed = platform.fixed;
    let (suspend_present, suspend_a, suspend_b) = c_sleep(platform.suspend);
    let (soft_off_present, soft_off_a, soft_off_b) = c_sleep(platform.soft_off);
    let (reset_present, reset_register, reset_value) = platform.fixed.reset.map_or(
        (false, c_optional(None).register_value, 0),
        |ResetRegister { register, value }| (true, c_register(register), value),
    );
    CPowerPlatform {
        pm1a_event: c_optional(fixed.pm1a_event),
        pm1b_event: c_optional(fixed.pm1b_event),
        pm1a_control: c_optional(fixed.pm1a_control),
        pm1b_control: c_optional(fixed.pm1b_control),
        pm1_event_bytes: fixed.pm1_event_bytes,
        smi_command_port: fixed.smi_command_port,
        acpi_enable_value: fixed.acpi_enable_value,
        reset_present,
        reset_register,
        reset_value,
        reduced_hardware: fixed.reduced_hardware,
        sleep_control: c_optional(fixed.sleep_control),
        sleep_status: c_optional(fixed.sleep_status),
        suspend_present,
        soft_off_present,
        suspend_a,
        suspend_b,
        soft_off_a,
        soft_off_b,
    }
}

fn enable_acpi(platform: &AcpiPlatform) -> Result<(), AcpiError> {
    let raw = c_platform(platform);
    // SAFETY: C reads the copied, repr(C) power platform record.
    map_power_error(unsafe { ghostos_power_enable_acpi(&raw, 1_000_000) })
}

fn poll_events(platform: &AcpiPlatform) -> Result<u16, AcpiError> {
    let raw = c_platform(platform);
    let mut events = 0;
    // SAFETY: C reads the copied platform record and writes one event mask.
    map_power_error(unsafe { ghostos_power_poll_events(&raw, &mut events) })?;
    Ok(events)
}

fn set_event(platform: &AcpiPlatform, event: u8, enabled: bool) -> Result<(), AcpiError> {
    let raw = c_platform(platform);
    // SAFETY: C reads the copied platform record and updates the requested event bit.
    map_power_error(unsafe { ghostos_power_set_event(&raw, event, enabled) })
}

fn request(platform: &AcpiPlatform, state: u8) -> Result<(), AcpiError> {
    let raw = c_platform(platform);
    // SAFETY: C reads the copied platform record and issues the selected ACPI request.
    map_power_error(unsafe { ghostos_power_request(&raw, state) })
}

fn map_power_error(error: i32) -> Result<(), AcpiError> {
    match error {
        0 => Ok(()),
        1 => Err(AcpiError::InvalidAddress),
        2 => Err(AcpiError::Unsupported),
        3 => Err(AcpiError::TimedOut),
        _ => Err(AcpiError::MalformedTable),
    }
}

pub fn suspend(platform: Option<&AcpiPlatform>) -> Result<(), AcpiError> {
    let platform = platform.ok_or(AcpiError::Unsupported)?;
    enable_acpi(platform)?;
    if !platform.fixed.reduced_hardware && platform.fixed.pm1a_event.is_some() {
        let _ = poll_events(platform)?;
        for event in [1, 2, 8] {
            set_event(platform, event, true)?
        }
    }

    crate::println!("Suspending GhostOS...");
    crate::arch::disable_interrupts();
    let requested = request(platform, 0);
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
    enable_acpi(platform)?;
    if platform.fixed.pm1a_event.is_some() {
        let _ = poll_events(platform)?;
    }
    Ok(())
}

pub fn shutdown(platform: Option<&AcpiPlatform>) -> ! {
    crate::println!("Shutting down GhostOS...");
    let acpi_requested = if let Some(platform) = platform {
        let _ = enable_acpi(platform);
        request(platform, 1).is_ok()
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
        let _ = enable_acpi(platform);
        request(platform, 2).is_ok()
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

#[cfg(test)]
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
