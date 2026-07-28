#![no_std]
#![no_main]

use core::ffi::c_void;
use synos_boot_protocol::{BootInfo, BootMethod, MemoryKind, MemoryRegion};

type EfiHandle = *mut c_void;
type EfiStatus = usize;
const EFI_SUCCESS: EfiStatus = 0;
const EFI_NOT_READY: EfiStatus = 0x8000_0000_0000_0006;
const MEMORY_MAP_CAPACITY: usize = 32 * 1024;

#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

type GetMemoryMap = unsafe extern "efiapi" fn(
    memory_map_size: *mut usize,
    memory_map: *mut u8,
    map_key: *mut usize,
    descriptor_size: *mut usize,
    descriptor_version: *mut u32,
) -> EfiStatus;

type ExitBootServices =
    unsafe extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus;

type TextReset =
    unsafe extern "efiapi" fn(output: *mut EfiSimpleTextOutput, extended: bool) -> EfiStatus;
type OutputString =
    unsafe extern "efiapi" fn(output: *mut EfiSimpleTextOutput, text: *const u16) -> EfiStatus;
type ClearScreen =
    unsafe extern "efiapi" fn(output: *mut EfiSimpleTextOutput) -> EfiStatus;
type InputReset =
    unsafe extern "efiapi" fn(input: *mut EfiSimpleTextInput, extended: bool) -> EfiStatus;
type ReadKeyStroke = unsafe extern "efiapi" fn(
    input: *mut EfiSimpleTextInput,
    key: *mut EfiInputKey,
) -> EfiStatus;

#[repr(C)]
struct EfiSimpleTextOutput {
    reset: TextReset,
    output_string: OutputString,
    test_string: usize,
    query_mode: usize,
    set_mode: usize,
    set_attribute: usize,
    clear_screen: ClearScreen,
    set_cursor_position: usize,
    enable_cursor: usize,
    mode: *mut c_void,
}

#[repr(C)]
struct EfiSimpleTextInput {
    reset: InputReset,
    read_key_stroke: ReadKeyStroke,
    wait_for_key: EfiHandle,
}

#[repr(C)]
struct EfiInputKey {
    scan_code: u16,
    unicode_char: u16,
}

#[repr(C)]
struct EfiBootServices {
    header: EfiTableHeader,
    before_get_memory_map: [usize; 4],
    get_memory_map: GetMemoryMap,
    before_exit_boot_services: [usize; 21],
    exit_boot_services: ExitBootServices,
}

#[repr(C)]
struct EfiSystemTable {
    header: EfiTableHeader,
    firmware_vendor: *const u16,
    firmware_revision: u32,
    console_in_handle: EfiHandle,
    console_in: *mut EfiSimpleTextInput,
    console_out_handle: EfiHandle,
    console_out: *mut EfiSimpleTextOutput,
    standard_error_handle: EfiHandle,
    standard_error: *mut c_void,
    runtime_services: *mut c_void,
    boot_services: *mut EfiBootServices,
}

#[repr(C)]
struct EfiMemoryDescriptor {
    memory_type: u32,
    padding: u32,
    physical_start: u64,
    virtual_start: u64,
    number_of_pages: u64,
    attributes: u64,
}

static mut MEMORY_MAP: [u8; MEMORY_MAP_CAPACITY] = [0; MEMORY_MAP_CAPACITY];
static mut BOOT_INFO: BootInfo = BootInfo::empty(BootMethod::Uefi);

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(image: EfiHandle, system_table: *mut EfiSystemTable) -> EfiStatus {
    if system_table.is_null() {
        return 1
    }

    // Safety: firmware provides valid tables for the lifetime of boot services.
    unsafe {
        let output = (*system_table).console_out;
        let input = (*system_table).console_in;
        let services = (*system_table).boot_services;
        if output.is_null() || input.is_null() || services.is_null() {
            return 1
        }

        ((*output).reset)(output, false);
        ((*output).clear_screen)(output);
        write_text(output, "SynOS bare-metal bootstrap\r\n");
        write_text(output, "==========================\r\n\r\n");
        write_text(output, "UEFI loader is ready.\r\n");
        write_text(output, "Press any key to boot SynOS...");
        wait_for_key(input);
        write_text(output, "\r\n\r\nPreparing memory map...\r\n");
        write_text(output, "Starting SynOS kernel...\r\n");
        write_text(output, "Firmware services will now stop.\r\n");

        let mut last_status = 1;
        for attempt in 0..4 {
            if attempt != 0 {
                write_text(output, "Memory map changed. Retrying handoff...\r\n");
            }

            let mut map_size = MEMORY_MAP_CAPACITY;
            let mut map_key = 0;
            let mut descriptor_size = 0;
            let mut descriptor_version = 0;
            let status = ((*services).get_memory_map)(
                &mut map_size,
                (&raw mut MEMORY_MAP).cast::<u8>(),
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            );
            if status != EFI_SUCCESS || descriptor_size < size_of::<EfiMemoryDescriptor>() {
                write_failure(output, "GetMemoryMap failed", status);
                wait_for_key(input);
                return status
            }

            let boot_info = &raw mut BOOT_INFO;
            *boot_info = BootInfo::empty(BootMethod::Uefi);
            fill_memory_map(&mut *boot_info, map_size, descriptor_size);

            last_status = ((*services).exit_boot_services)(image, map_key);
            if last_status == EFI_SUCCESS {
                synos_kernel::kernel_entry(&*boot_info)
            }
        }

        write_failure(output, "ExitBootServices failed", last_status);
        wait_for_key(input);
        last_status
    }
}

unsafe fn write_text(output: *mut EfiSimpleTextOutput, text: &str) {
    let mut buffer = [0u16; 128];
    let mut used = 0;

    for byte in text.bytes() {
        if used == buffer.len() - 1 {
            unsafe {
                ((*output).output_string)(output, buffer.as_ptr());
            }
            buffer = [0; 128];
            used = 0;
        }
        buffer[used] = byte as u16;
        used += 1;
    }

    if used != 0 {
        unsafe {
            ((*output).output_string)(output, buffer.as_ptr());
        }
    }
}

unsafe fn write_failure(output: *mut EfiSimpleTextOutput, message: &str, status: EfiStatus) {
    unsafe {
        write_text(output, "\r\nSynOS boot failure: ");
        write_text(output, message);
        write_text(output, "\r\nEFI status: 0x");

        let mut hex = [0u16; 17];
        for index in 0..16 {
            let shift = (15 - index) * 4;
            let digit = ((status >> shift) & 0xf) as u8;
            hex[index] = match digit {
                0..=9 => (b'0' + digit) as u16,
                _ => (b'A' + digit - 10) as u16,
            };
        }
        ((*output).output_string)(output, hex.as_ptr());
        write_text(output, "\r\nPress any key to return to firmware.\r\n");
    }
}

unsafe fn wait_for_key(input: *mut EfiSimpleTextInput) {
    unsafe {
        ((*input).reset)(input, false);
    }

    loop {
        let mut key = EfiInputKey {
            scan_code: 0,
            unicode_char: 0,
        };
        let status = unsafe { ((*input).read_key_stroke)(input, &mut key) };
        if status == EFI_SUCCESS {
            return
        }
        if status != EFI_NOT_READY {
            return
        }
        core::hint::spin_loop()
    }
}

unsafe fn fill_memory_map(boot_info: &mut BootInfo, map_size: usize, descriptor_size: usize) {
    let descriptor_count = map_size / descriptor_size;

    for index in 0..descriptor_count {
        let address = unsafe {
            (&raw const MEMORY_MAP)
                .cast::<u8>()
                .add(index * descriptor_size)
                .cast::<EfiMemoryDescriptor>()
        };
        let descriptor = unsafe { &*address };
        let region = MemoryRegion {
            start: descriptor.physical_start,
            length: descriptor.number_of_pages.saturating_mul(4096),
            kind: memory_kind(descriptor.memory_type),
            attributes: descriptor.attributes as u32,
        };

        if !boot_info.push_region(region) {
            break
        }
    }
}

const fn memory_kind(efi_type: u32) -> MemoryKind {
    match efi_type {
        3 | 4 | 7 => MemoryKind::Usable,
        9 => MemoryKind::AcpiReclaimable,
        10 => MemoryKind::AcpiNonVolatile,
        1 | 2 => MemoryKind::Bootloader,
        _ => MemoryKind::Reserved,
    }
}
