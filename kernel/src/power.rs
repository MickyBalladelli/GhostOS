use core::ptr::{read_volatile, write_volatile};

use ghostos_boot_protocol::{BootInfo, MemoryKind};
use ghostos_power::{
    AcpiError, AcpiMemory, AcpiPlatform, AddressSpace, FixedEvent, GenericAddress,
    PowerController, PowerIo, PowerState,
};

#[cfg(target_arch = "x86_64")]
const VM_POWER_CONTROL_PORT: u16 = 0x604;
#[cfg(target_arch = "x86_64")]
const VM_REBOOT_VALUE: u16 = 1 << 13;
#[cfg(target_arch = "x86_64")]
const VM_SOFT_OFF_VALUE: u16 = (5 << 10) | (1 << 13);

struct PhysicalAcpiMemory<'a> {
    boot_info: &'a BootInfo,
}

impl AcpiMemory for PhysicalAcpiMemory<'_> {
    fn read(
        &self,
        physical_address: u64,
        destination: &mut [u8],
    ) -> Result<(), AcpiError> {
        let end = physical_address
            .checked_add(destination.len() as u64)
            .ok_or(AcpiError::InvalidAddress)?;
        let mapped = self.boot_info.regions().iter().any(|region| {
            region.kind != MemoryKind::Usable
                && physical_address >= region.start
                && end <= region.end()
        });
        if !mapped {
            return Err(AcpiError::InvalidAddress)
        }
        let virtual_address = physical_address
            .checked_add(self.boot_info.physical_address_offset)
            .ok_or(AcpiError::InvalidAddress)?;
        let source = usize::try_from(virtual_address)
            .map_err(|_| AcpiError::InvalidAddress)? as *const u8;
        unsafe {
            core::ptr::copy_nonoverlapping(
                source,
                destination.as_mut_ptr(),
                destination.len(),
            )
        }
        Ok(())
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

    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let mut attempts = 100_000;
        while attempts != 0 && in_u8(0x64) & 0x02 != 0 {
            attempts -= 1;
            core::hint::spin_loop()
        }
        out_u8(0x64, 0xfe)
    }
    crate::halt()
}

struct PlatformIo;

fn request_vm_reboot() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        out_u16(VM_POWER_CONTROL_PORT, VM_REBOOT_VALUE)
    }
}

fn request_vm_shutdown() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        out_u16(VM_POWER_CONTROL_PORT, VM_SOFT_OFF_VALUE)
    }
}

impl PowerIo for PlatformIo {
    fn read(&mut self, register: GenericAddress) -> Result<u64, AcpiError> {
        let bytes = access_bytes(register)?;
        let raw = match register.address_space {
            AddressSpace::SystemMemory => unsafe {
                read_memory(register.address, bytes)?
            },
            AddressSpace::SystemIo => unsafe { read_io(register.address, bytes)? },
            _ => return Err(AcpiError::Unsupported),
        };
        Ok(extract_field(raw, register))
    }

    fn write(
        &mut self,
        register: GenericAddress,
        value: u64,
    ) -> Result<(), AcpiError> {
        let bytes = access_bytes(register)?;
        let shifted = value
            .checked_shl(register.bit_offset as u32)
            .ok_or(AcpiError::InvalidAddress)?;
        match register.address_space {
            AddressSpace::SystemMemory => unsafe {
                write_memory(register.address, bytes, shifted)
            },
            AddressSpace::SystemIo => unsafe { write_io(register.address, bytes, shifted) },
            _ => Err(AcpiError::Unsupported),
        }
    }
}

fn access_bytes(register: GenericAddress) -> Result<u8, AcpiError> {
    let bytes = match register.access_size {
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        0 => match register.bit_width.saturating_add(register.bit_offset) {
            0..=8 => 1,
            9..=16 => 2,
            17..=32 => 4,
            _ => 8,
        },
        _ => return Err(AcpiError::Unsupported),
    };
    if register.address == 0 || register.address % bytes as u64 != 0 {
        Err(AcpiError::InvalidAddress)
    } else {
        Ok(bytes)
    }
}

fn extract_field(raw: u64, register: GenericAddress) -> u64 {
    let shifted = raw >> register.bit_offset;
    if register.bit_width >= 64 {
        shifted
    } else {
        shifted & ((1_u64 << register.bit_width) - 1)
    }
}

unsafe fn read_memory(address: u64, bytes: u8) -> Result<u64, AcpiError> {
    let pointer =
        usize::try_from(address).map_err(|_| AcpiError::InvalidAddress)? as *const u8;
    Ok(unsafe {
        match bytes {
            1 => read_volatile(pointer) as u64,
            2 => read_volatile(pointer.cast::<u16>()) as u64,
            4 => read_volatile(pointer.cast::<u32>()) as u64,
            8 => read_volatile(pointer.cast::<u64>()),
            _ => return Err(AcpiError::Unsupported),
        }
    })
}

unsafe fn write_memory(
    address: u64,
    bytes: u8,
    value: u64,
) -> Result<(), AcpiError> {
    let pointer =
        usize::try_from(address).map_err(|_| AcpiError::InvalidAddress)? as *mut u8;
    unsafe {
        match bytes {
            1 => write_volatile(pointer, value as u8),
            2 => write_volatile(pointer.cast::<u16>(), value as u16),
            4 => write_volatile(pointer.cast::<u32>(), value as u32),
            8 => write_volatile(pointer.cast::<u64>(), value),
            _ => return Err(AcpiError::Unsupported),
        }
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_io(address: u64, bytes: u8) -> Result<u64, AcpiError> {
    let port = u16::try_from(address).map_err(|_| AcpiError::InvalidAddress)?;
    Ok(unsafe {
        match bytes {
            1 => in_u8(port) as u64,
            2 => in_u16(port) as u64,
            4 => in_u32(port) as u64,
            _ => return Err(AcpiError::Unsupported),
        }
    })
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn read_io(_address: u64, _bytes: u8) -> Result<u64, AcpiError> {
    Err(AcpiError::Unsupported)
}

#[cfg(target_arch = "x86_64")]
unsafe fn write_io(address: u64, bytes: u8, value: u64) -> Result<(), AcpiError> {
    let port = u16::try_from(address).map_err(|_| AcpiError::InvalidAddress)?;
    unsafe {
        match bytes {
            1 => out_u8(port, value as u8),
            2 => out_u16(port, value as u16),
            4 => out_u32(port, value as u32),
            _ => return Err(AcpiError::Unsupported),
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn write_io(_address: u64, _bytes: u8, _value: u64) -> Result<(), AcpiError> {
    Err(AcpiError::Unsupported)
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

#[cfg(target_arch = "x86_64")]
unsafe fn in_u8(port: u16) -> u8 {
    let value;
    unsafe {
        core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack))
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u16(port: u16) -> u16 {
    let value;
    unsafe {
        core::arch::asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack))
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u32(port: u16) -> u32 {
    let value;
    unsafe {
        core::arch::asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack))
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn out_u8(port: u16, value: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack))
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn out_u16(port: u16, value: u16) {
    unsafe {
        core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack))
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn out_u32(port: u16, value: u32) {
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack))
    }
}
