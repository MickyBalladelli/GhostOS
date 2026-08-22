#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

mod chainload;

use core::ffi::c_void;
use core::panic::PanicInfo;
use ghostos_boot_protocol::{
    BootInfo, BootMethod, FRAMEBUFFER_PIXEL_BGR, FRAMEBUFFER_PIXEL_RGB,
    FramebufferInfo, MemoryKind, MemoryRegion,
};

pub(crate) type EfiHandle = *mut c_void;
pub(crate) type EfiStatus = usize;
pub(crate) const EFI_SUCCESS: EfiStatus = 0;
const EFI_NOT_READY: EfiStatus = 0x8000_0000_0000_0006;
const EFI_DEVICE_ERROR: EfiStatus = 0x8000_0000_0000_0007;
const EFI_OUT_OF_RESOURCES: EfiStatus = 0x8000_0000_0000_0009;
const EFI_NOT_FOUND: EfiStatus = 0x8000_0000_0000_000e;
const MEMORY_MAP_CAPACITY: usize = 32 * 1024;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    ghostos_kernel::panic_report(info)
}

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
type AllocatePool = unsafe extern "efiapi" fn(
    pool_type: u32,
    size: usize,
    buffer: *mut *mut c_void,
) -> EfiStatus;

type ExitBootServices =
    unsafe extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus;
pub(crate) type HandleProtocol = unsafe extern "efiapi" fn(
    handle: EfiHandle,
    protocol: *const EfiGuid,
    interface: *mut *mut c_void,
) -> EfiStatus;
pub(crate) type FreePool = unsafe extern "efiapi" fn(buffer: *mut c_void) -> EfiStatus;
pub(crate) type LocateHandleBuffer = unsafe extern "efiapi" fn(
    search_type: u32,
    protocol: *const EfiGuid,
    search_key: *mut c_void,
    handle_count: *mut usize,
    handles: *mut *mut EfiHandle,
) -> EfiStatus;
pub(crate) type LoadImage = unsafe extern "efiapi" fn(
    boot_policy: bool,
    parent_image_handle: EfiHandle,
    device_path: *const c_void,
    source_buffer: *const c_void,
    source_size: usize,
    image_handle: *mut EfiHandle,
) -> EfiStatus;
pub(crate) type StartImage = unsafe extern "efiapi" fn(
    image_handle: EfiHandle,
    exit_data_size: *mut usize,
    exit_data: *mut *mut u16,
) -> EfiStatus;
pub(crate) type UnloadImage = unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus;
type LocateProtocol = unsafe extern "efiapi" fn(
    protocol: *const EfiGuid,
    registration: *mut c_void,
    interface: *mut *mut c_void,
) -> EfiStatus;

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
pub(crate) struct EfiInputKey {
    scan_code: u16,
    unicode_char: u16,
}

#[repr(C)]
pub(crate) struct EfiBootServices {
    header: EfiTableHeader,
    before_get_memory_map: [usize; 4],
    get_memory_map: GetMemoryMap,
    allocate_pool: AllocatePool,
    pub(crate) free_pool: FreePool,
    create_event: usize,
    set_timer: usize,
    wait_for_event: usize,
    signal_event: usize,
    close_event: usize,
    check_event: usize,
    install_protocol_interface: usize,
    reinstall_protocol_interface: usize,
    uninstall_protocol_interface: usize,
    pub(crate) handle_protocol: HandleProtocol,
    reserved: usize,
    register_protocol_notify: usize,
    locate_handle: usize,
    locate_device_path: usize,
    install_configuration_table: usize,
    pub(crate) load_image: LoadImage,
    pub(crate) start_image: StartImage,
    exit: usize,
    pub(crate) unload_image: UnloadImage,
    exit_boot_services: ExitBootServices,
    get_next_monotonic_count: usize,
    stall: usize,
    set_watchdog_timer: usize,
    connect_controller: usize,
    disconnect_controller: usize,
    open_protocol: usize,
    close_protocol: usize,
    open_protocol_information: usize,
    protocols_per_handle: usize,
    pub(crate) locate_handle_buffer: LocateHandleBuffer,
    locate_protocol: LocateProtocol,
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
    number_of_table_entries: usize,
    configuration_table: *const EfiConfigurationTable,
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

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct EfiGuid {
    pub(crate) data1: u32,
    pub(crate) data2: u16,
    pub(crate) data3: u16,
    pub(crate) data4: [u8; 8],
}

#[repr(C)]
struct EfiConfigurationTable {
    vendor_guid: EfiGuid,
    vendor_table: *const c_void,
}

#[repr(C)]
struct EfiGraphicsOutput {
    query_mode: usize,
    set_mode: usize,
    blt: usize,
    mode: *mut EfiGraphicsOutputMode,
}

#[repr(C)]
struct EfiGraphicsOutputMode {
    max_mode: u32,
    mode: u32,
    info: *mut EfiGraphicsOutputModeInfo,
    size_of_info: usize,
    framebuffer_base: u64,
    framebuffer_size: usize,
}

#[repr(C)]
struct EfiGraphicsOutputModeInfo {
    version: u32,
    horizontal_resolution: u32,
    vertical_resolution: u32,
    pixel_format: u32,
    pixel_information: [u32; 4],
    pixels_per_scan_line: u32,
}

const GRAPHICS_OUTPUT_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x9042_a9de,
    data2: 0x23dc,
    data3: 0x4a38,
    data4: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};
const ACPI_20_TABLE: EfiGuid = EfiGuid {
    data1: 0x8868_e871,
    data2: 0xe4f1,
    data3: 0x11d3,
    data4: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};
const ACPI_10_TABLE: EfiGuid = EfiGuid {
    data1: 0xeb9d_2d30,
    data2: 0x2d88,
    data3: 0x11d3,
    data4: [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d],
};
const LOADED_IMAGE_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x5b1b_31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
const BLOCK_IO_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x964e_5b21,
    data2: 0x6459,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

#[repr(C)]
struct EfiLoadedImage {
    revision: u32,
    parent_image_handle: EfiHandle,
    system_table: *mut c_void,
    device_handle: EfiHandle,
    file_path: *mut c_void,
    reserved: *mut c_void,
    load_options_size: u32,
    load_options: *mut c_void,
    image_base: *mut c_void,
    image_size: u64,
    image_code_type: u32,
    image_data_type: u32,
    unload: usize,
}

#[repr(C)]
struct EfiBlockIo {
    revision: u64,
    media: *mut EfiBlockIoMedia,
    reset: usize,
    read_blocks: unsafe extern "efiapi" fn(
        this: *mut EfiBlockIo,
        media_id: u32,
        lba: u64,
        buffer_size: usize,
        buffer: *mut c_void,
    ) -> EfiStatus,
    write_blocks: usize,
    flush_blocks: usize,
}

#[repr(C)]
struct EfiBlockIoMedia {
    media_id: u32,
    removable_media: bool,
    media_present: bool,
    logical_partition: bool,
    read_only: bool,
    write_caching: bool,
    block_size: u32,
    io_align: u32,
    last_block: u64,
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
        loop {
            write_text(output, "GhostOS boot manager\r\n");
            write_text(output, "==================\r\n\r\n");
            write_text(output, "1  Boot GhostOS\r\n");
            write_text(output, "2  Windows Boot Manager\r\n");
            write_text(output, "3  GRUB\r\n\r\n");
            write_text(output, "Choose 1, 2, or 3: ");

            match wait_for_choice(input) {
                '1' => {
                    ((*output).clear_screen)(output);
                    break
                }
                '2' => {
                    write_text(output, "\r\nStarting Windows Boot Manager...\r\n");
                    let status = chainload::windows(image, services);
                    write_chainload_result(output, input, status)
                }
                '3' => {
                    write_text(output, "\r\nSearching for GRUB...\r\n");
                    let status = chainload::grub(image, services);
                    write_chainload_result(output, input, status)
                }
                _ => {}
            }

            ((*output).clear_screen)(output);
        }

        match measure_installed_system(image, services) {
            EFI_SUCCESS | EFI_NOT_FOUND => {}
            status => {
                write_failure(output, "installed system image validation failed", status);
                wait_for_key(input);
                return status
            }
        }

        let framebuffer = locate_framebuffer(services);
        let rsdp_address = locate_rsdp(system_table);

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
            (*boot_info).framebuffer = framebuffer;
            (*boot_info).rsdp_address = rsdp_address;
            fill_memory_map(&mut *boot_info, map_size, descriptor_size);

            last_status = ((*services).exit_boot_services)(image, map_key);
            if last_status == EFI_SUCCESS {
                ghostos_kernel::kernel_entry(&*boot_info)
            }
        }

        write_failure(output, "ExitBootServices failed", last_status);
        wait_for_key(input);
        last_status
    }
}

unsafe fn measure_installed_system(
    image: EfiHandle,
    services: *mut EfiBootServices,
) -> EfiStatus {
    let mut loaded_interface = core::ptr::null_mut();
    let status = unsafe {
        ((*services).handle_protocol)(
            image,
            &LOADED_IMAGE_PROTOCOL,
            &mut loaded_interface,
        )
    };
    if status != EFI_SUCCESS || loaded_interface.is_null() {
        return EFI_NOT_FOUND
    }

    let device = unsafe { (*loaded_interface.cast::<EfiLoadedImage>()).device_handle };
    let mut block_interface = core::ptr::null_mut();
    let status = unsafe {
        ((*services).handle_protocol)(device, &BLOCK_IO_PROTOCOL, &mut block_interface)
    };
    if status != EFI_SUCCESS || block_interface.is_null() {
        return EFI_NOT_FOUND
    }

    let block = block_interface.cast::<EfiBlockIo>();
    let media = unsafe { (*block).media };
    if media.is_null() || unsafe { (*media).block_size } != 512 {
        return EFI_NOT_FOUND
    }

    let media_id = unsafe { (*media).media_id };
    let mut record = [0u8; 512];
    let status = unsafe {
        ((*block).read_blocks)(
            block,
            media_id,
            128,
            record.len(),
            record.as_mut_ptr().cast(),
        )
    };
    if status != EFI_SUCCESS {
        return EFI_NOT_FOUND
    }
    if &record[..8] != b"SYNBOOT1"
        || get_u32(&record, 8) != 1
        || get_u32(&record, 508) != boot_record_checksum(&record)
    {
        return EFI_NOT_FOUND
    }

    let kernel_status = unsafe {
        read_measured_extent(
            block,
            media_id,
            services,
            get_u64(&record, 20),
            get_u64(&record, 28),
            get_u32(&record, 52),
        )
    };
    if kernel_status != EFI_SUCCESS {
        return kernel_status
    }
    unsafe {
        read_measured_extent(
            block,
            media_id,
            services,
            get_u64(&record, 36),
            get_u64(&record, 44),
            get_u32(&record, 56),
        )
    }
}

unsafe fn read_measured_extent(
    block: *mut EfiBlockIo,
    media_id: u32,
    services: *mut EfiBootServices,
    offset: u64,
    size: u64,
    expected: u32,
) -> EfiStatus {
    if size == 0 {
        return if expected == 0 { EFI_SUCCESS } else { EFI_DEVICE_ERROR }
    }
    if offset % 512 != 0 {
        return EFI_DEVICE_ERROR
    }
    let sectors = size.div_ceil(512);
    let buffer_size = match sectors.checked_mul(512).and_then(|value| usize::try_from(value).ok()) {
        Some(value) => value,
        None => return EFI_OUT_OF_RESOURCES,
    };
    let mut buffer = core::ptr::null_mut();
    let status = unsafe { ((*services).allocate_pool)(2, buffer_size, &mut buffer) };
    if status != EFI_SUCCESS || buffer.is_null() {
        return status
    }
    let status = unsafe {
        ((*block).read_blocks)(
            block,
            media_id,
            offset / 512,
            buffer_size,
            buffer,
        )
    };
    if status != EFI_SUCCESS {
        unsafe { ((*services).free_pool)(buffer) };
        return status
    }
    let bytes = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) };
    let measured = crc32(bytes);
    unsafe { ((*services).free_pool)(buffer) };
    if measured == expected { EFI_SUCCESS } else { EFI_DEVICE_ERROR }
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut value = [0u8; 4];
    value.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(value)
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

fn boot_record_checksum(bytes: &[u8]) -> u32 {
    bytes[..bytes.len() - 4]
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(u32::from(*byte)))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

unsafe fn locate_rsdp(system_table: *const EfiSystemTable) -> u64 {
    let count = unsafe { (*system_table).number_of_table_entries }.min(4096);
    let tables = unsafe { (*system_table).configuration_table };
    if tables.is_null() {
        return 0
    }
    let mut acpi_v1 = 0;
    for index in 0..count {
        let table = unsafe { &*tables.add(index) };
        if table.vendor_guid == ACPI_20_TABLE {
            return table.vendor_table as u64
        }
        if table.vendor_guid == ACPI_10_TABLE {
            acpi_v1 = table.vendor_table as u64
        }
    }
    acpi_v1
}

unsafe fn locate_framebuffer(services: *mut EfiBootServices) -> FramebufferInfo {
    let mut interface = core::ptr::null_mut();
    let status = unsafe {
        ((*services).locate_protocol)(
            &GRAPHICS_OUTPUT_PROTOCOL,
            core::ptr::null_mut(),
            &mut interface,
        )
    };
    if status != EFI_SUCCESS || interface.is_null() {
        return FramebufferInfo::EMPTY
    }

    let graphics = interface.cast::<EfiGraphicsOutput>();
    let mode = unsafe { (*graphics).mode };
    if mode.is_null() {
        return FramebufferInfo::EMPTY
    }
    let info = unsafe { (*mode).info };
    if info.is_null() || unsafe { (*mode).framebuffer_base } == 0 {
        return FramebufferInfo::EMPTY
    }

    let pixel_format = match unsafe { (*info).pixel_format } {
        0 => FRAMEBUFFER_PIXEL_RGB,
        1 => FRAMEBUFFER_PIXEL_BGR,
        _ => return FramebufferInfo::EMPTY,
    };
    FramebufferInfo {
        address: unsafe { (*mode).framebuffer_base },
        size: unsafe { (*mode).framebuffer_size as u64 },
        width: unsafe { (*info).horizontal_resolution },
        height: unsafe { (*info).vertical_resolution },
        stride: unsafe { (*info).pixels_per_scan_line },
        pixel_format,
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
        write_text(output, "\r\nGhostOS boot failure: ");
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

unsafe fn write_chainload_result(
    output: *mut EfiSimpleTextOutput,
    input: *mut EfiSimpleTextInput,
    status: EfiStatus,
) {
    unsafe {
        if status == EFI_SUCCESS {
            write_text(output, "\r\nBoot target returned to GhostOS.")
        } else {
            write_text(output, "\r\nCould not start boot target.\r\nEFI status: 0x");
            write_hex(output, status)
        }
        write_text(output, "\r\nPress any key for the GhostOS menu.\r\n");
        wait_for_key(input)
    }
}

unsafe fn write_hex(output: *mut EfiSimpleTextOutput, value: usize) {
    let mut hex = [0u16; 17];
    for index in 0..16 {
        let shift = (15 - index) * 4;
        let digit = ((value >> shift) & 0xf) as u8;
        hex[index] = match digit {
            0..=9 => (b'0' + digit) as u16,
            _ => (b'A' + digit - 10) as u16,
        };
    }
    unsafe {
        ((*output).output_string)(output, hex.as_ptr());
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

unsafe fn wait_for_choice(input: *mut EfiSimpleTextInput) -> char {
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
            return char::from_u32(key.unicode_char as u32).unwrap_or('\0')
        }
        if status != EFI_NOT_READY {
            return '\0'
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
