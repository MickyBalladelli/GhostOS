//! UEFI firmware: SystemTable, boot services, runtime services, and EFI
//! application (PE32+) loading.
//!
//! The GhostOS boot manager (`boot/uefi`) is a real UEFI application. When it
//! runs in this VM it finds the `EFI_SYSTEM_TABLE` handed to `efi_main`,
//! prints a menu through `ConOut`, reads a key through `ConIn`, queries the
//! memory map and GOP framebuffer, then calls `ExitBootServices` before the
//! kernel handoff.
//!
//! Design notes
//! ============
//!
//! * Firmware tables are materialized in guest RAM so the bootloader can
//!   dereference them natively. All pointers are identity-mapped physical
//!   addresses (UEFI starts in flat 64-bit mode).
//!
//! * Boot services are *stubs*. Each service in the tables points to a tiny
//!   machine-code stub we emit into guest RAM:
//!
//!   ```text
//!   B8 <id:u32>      mov eax, <service id>
//!   CD E0            int  0xE0        ; firmware call vector
//!   C3               ret
//!   ```
//!
//!   The CPU is real, so the stub returns to the guest with the status in
//!   RAX. The executor already routes unmapped INT vectors to
//!   `BiosContext::call_int`, which forwards vector 0xE0 to
//!   `UefiContext::dispatch`. This avoids adding any new instruction
//!   emulation while still letting the bootloader execute natively.
//!
//! * Arguments follow the Microsoft x64 ABI used by `extern "efiapi"`:
//!   RCX, RDX, R8, R9 for the first four arguments and the fifth argument
//!   onward at `[RSP + 0x28]`, `[RSP + 0x30]`, ... at stub entry.

use crate::cpu::{CpuMode, CpuState};
use crate::devices::{DisplayState, VGA_COLS, VGA_ROWS, VGA_TEXT_BASE};
use crate::memory::{Mmu, PageFlags};
use crate::replay::SharedReplay;
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Firmware call vector and guest memory layout
// ---------------------------------------------------------------------------

/// Dedicated interrupt vector used for firmware service dispatch. The BIOS
/// path only handles 0x10/0x13/0x15/0x16/0x19 so 0xE0 is free in both modes.
pub const UEFI_CALL_VECTOR: u8 = 0xE0;

/// Top of the 64 KiB firmware stack (grows down).
pub const UEFI_STACK_TOP: u64 = 0x0100_0000;
/// Base of the guest-resident firmware table blob.
pub const UEFI_TABLES_BASE: u64 = 0x0120_0000;
/// Guest buffer used by GetMemoryMap.
pub const UEFI_MEMORY_MAP_BASE: u64 = 0x0180_0000;
pub const UEFI_MEMORY_MAP_SIZE: usize = 32 * 1024;
/// Load address for the primary EFI application.
pub const UEFI_IMAGE_BASE: u64 = 0x0200_0000;
/// Load address for chainloaded (secondary) images.
pub const UEFI_CHILD_IMAGE_BASE: u64 = 0x0600_0000;

/// Reserved firmware real-estate that starts at 1 MiB.
const UEFI_FIRMWARE_START: u64 = 0x100000;

/// Token handles (non-null, unique pointer values).
const DEVICE_HANDLE: u64 = 0x0122_0000;
const CONSOLE_IN_HANDLE: u64 = 0x0122_1000;
const CONSOLE_OUT_HANDLE: u64 = 0x0122_2000;

/// Offsets relative to `UEFI_TABLES_BASE`.
const TBL_SYSTEM_TABLE: u64 = 0x000;
const TBL_BOOT_SERVICES: u64 = 0x200;
const TBL_RUNTIME_SERVICES: u64 = 0x400;
const TBL_CONSOLE_OUT: u64 = 0x500;
const TBL_CONSOLE_IN: u64 = 0x560;
const TBL_CONSOLE_MODE: u64 = 0x580;
const TBL_CONFIG_TABLE: u64 = 0x600;
const TBL_RSDP: u64 = 0x680;
const TBL_STUBS: u64 = 0x800;
const TBL_GOP: u64 = 0xA00;
const TBL_GOP_MODE: u64 = 0xA40;
const TBL_GOP_INFO: u64 = 0xA80;
const TBL_FIRMWARE_VENDOR: u64 = 0xAC0;
const TBL_DEVICE_PATH: u64 = 0xB00;
const TBL_FILE_SYSTEM: u64 = 0xB10;
const TBL_IMAGE_FILE_PATH: u64 = 0xB20;
const TBL_LOADED_IMAGE: u64 = 0xC00; // up to 8 slots of 0x100
const TBL_HANDLE_BUFFER: u64 = 0x1500;
const TBL_RETURN_STUB: u64 = 0x1780;
const MAX_IMAGES: usize = 8;

// ---------------------------------------------------------------------------
// EFI status codes
// ---------------------------------------------------------------------------

pub const EFI_SUCCESS: u64 = 0;
pub const EFI_LOAD_ERROR: u64 = 0x8000_0000_0000_0001;
pub const EFI_INVALID_PARAMETER: u64 = 0x8000_0000_0000_0002;
pub const EFI_UNSUPPORTED: u64 = 0x8000_0000_0000_0003;
pub const EFI_BAD_BUFFER_SIZE: u64 = 0x8000_0000_0000_0004;
pub const EFI_BUFFER_TOO_SMALL: u64 = 0x8000_0000_0000_0005;
pub const EFI_NOT_READY: u64 = 0x8000_0000_0000_0006;
pub const EFI_DEVICE_ERROR: u64 = 0x8000_0000_0000_0007;
pub const EFI_OUT_OF_RESOURCES: u64 = 0x8000_0000_0000_0009;
pub const EFI_NOT_FOUND: u64 = 0x8000_0000_0000_000E;

// ---------------------------------------------------------------------------
// Service IDs (dispatched through `int 0xE0`)
// ---------------------------------------------------------------------------

const SERVICE_OUTPUT_STRING: u64 = 0;
const SERVICE_CLEAR_SCREEN: u64 = 1;
const SERVICE_RESET_OUTPUT: u64 = 2;
const SERVICE_READ_KEYSTROKE: u64 = 3;
const SERVICE_RESET_INPUT: u64 = 4;
const SERVICE_GET_MEMORY_MAP: u64 = 5;
const SERVICE_EXIT_BOOT_SERVICES: u64 = 6;
const SERVICE_LOCATE_PROTOCOL: u64 = 7;
const SERVICE_HANDLE_PROTOCOL: u64 = 8;
const SERVICE_LOCATE_HANDLE_BUFFER: u64 = 9;
const SERVICE_FREE_POOL: u64 = 10;
const SERVICE_LOAD_IMAGE: u64 = 11;
const SERVICE_START_IMAGE: u64 = 12;
const SERVICE_UNLOAD_IMAGE: u64 = 13;
const SERVICE_GET_VARIABLE: u64 = 14;
const SERVICE_SET_VARIABLE: u64 = 15;
const SERVICE_GET_NEXT_VARIABLE: u64 = 16;
const SERVICE_GET_TIME: u64 = 17;
const SERVICE_RESET_SYSTEM: u64 = 18;
const SERVICE_UNSUPPORTED: u64 = 99;

// ---------------------------------------------------------------------------
// Protocol GUIDs (stored little-endian, matching the guest EfiGuid layout)
// ---------------------------------------------------------------------------

/// EFI_GRAPHICS_OUTPUT_PROTOCOL
const GOP_GUID: [u8; 16] = [
    0xde, 0xa9, 0x42, 0x90, 0xdc, 0x23, 0x38, 0x4a, 0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a,
];
/// EFI_LOADED_IMAGE_PROTOCOL
const LOADED_IMAGE_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b, 0x62, 0x95, 0xd2, 0x11, 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];
/// EFI_DEVICE_PATH_PROTOCOL
const DEVICE_PATH_GUID: [u8; 16] = [
    0x91, 0x6e, 0x57, 0x09, 0x3f, 0x6d, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];
/// EFI_SIMPLE_FILE_SYSTEM_PROTOCOL
const FILE_SYSTEM_GUID: [u8; 16] = [
    0x22, 0x5b, 0x4e, 0x96, 0x59, 0x64, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];
/// ACPI 2.0 configuration-table GUID
const ACPI_20_GUID: [u8; 16] = [
    0x71, 0xe8, 0x68, 0x88, 0xf1, 0xe4, 0xd3, 0x11, 0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81,
];
/// ACPI 1.0 configuration-table GUID
const ACPI_10_GUID: [u8; 16] = [
    0x30, 0x2d, 0x9d, 0xeb, 0x88, 0x2d, 0xd3, 0x11, 0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d,
];

const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5459_5353_2049_4249;
const EFI_BOOT_SIGNATURE: u64 = 0x4259_5353_2049_4249;
const EFI_RUNTIME_SIGNATURE: u64 = 0x5259_5353_2049_4249;
const EFI_TABLE_REVISION: u32 = 0x0002_000F;
const EFI_TABLE_HEADER_SIZE: u32 = 24;

// ---------------------------------------------------------------------------
// Memory-map descriptor and PE image model
// ---------------------------------------------------------------------------

/// 48-byte EFI memory descriptor (UEFI >= 2.3 common descriptor stride).
const MEMORY_DESCRIPTOR_SIZE: usize = 48;

const MEM_TYPE_RESERVED: u32 = 0;
const MEM_TYPE_LOADER_CODE: u32 = 1;
const MEM_TYPE_LOADER_DATA: u32 = 2;
const MEM_TYPE_CONVENTIONAL: u32 = 7;

fn mem_desc(ty: u32, start: u64, len: u64, attr: u64) -> [u8; MEMORY_DESCRIPTOR_SIZE] {
    let mut d = [0u8; MEMORY_DESCRIPTOR_SIZE];
    d[0..4].copy_from_slice(&ty.to_le_bytes());
    d[8..16].copy_from_slice(&start.to_le_bytes());
    d[16..24].copy_from_slice(&start.to_le_bytes());
    d[24..32].copy_from_slice(&(len / 4096).to_le_bytes());
    d[32..40].copy_from_slice(&attr.to_le_bytes());
    d
}

struct PeSection {
    virtual_address: u32,
    raw_data: Vec<u8>,
}

struct PeImage {
    image_base: u64,
    entry_rva: u32,
    size_of_image: u32,
    reloc_rva: u32,
    reloc_size: u32,
    sections: Vec<PeSection>,
}

struct UefiImage {
    handle: u64,
    base: u64,
    entry: u64,
    size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UefiState {
    Reset,
    Initialized,
    Running,
    ExitBootServices,
    Halted,
}

impl Default for UefiState {
    fn default() -> Self {
        UefiState::Reset
    }
}

#[derive(Debug)]
pub enum UefiError {
    InitFailed,
    InvalidState,
    InvalidImage,
    OutOfMemory,
    LoadFailed,
    Unsupported,
}

// ---------------------------------------------------------------------------
// UEFI context
// ---------------------------------------------------------------------------

pub struct UefiContext {
    pub state: UefiState,
    memory_size: usize,
    display: Rc<RefCell<DisplayState>>,
    system_table: u64,
    boot_services: u64,
    runtime_services: u64,
    gop_interface: u64,
    device_handle: u64,
    /// EFI application payload returned by LoadImage / loaded at init.
    efi_app: Option<Vec<u8>>,
    /// Host-side image bookkeeping (LoadedImage protocol instances).
    images: Vec<UefiImage>,
    /// GetMemoryMap / ExitBootServices handshake token.
    map_key: usize,
    boot_services_active: bool,
    /// Key input queue (auto-seeded with '1' so the boot menu proceeds).
    pending_keys: Vec<u16>,
    /// Text captured from ConOut.OutputString.
    console: String,
    console_x: u8,
    console_y: u8,
    /// Runtime variable store.
    variables: Vec<(Vec<u16>, [u8; 16], Vec<u8>, u32)>,
    replay: Option<SharedReplay>,
    replay_instruction_ip: Option<u64>,
}

impl Default for UefiContext {
    fn default() -> Self {
        Self::new()
    }
}

impl UefiContext {
    pub fn new() -> Self {
        Self {
            state: UefiState::Reset,
            memory_size: 128 * 1024 * 1024,
            display: Rc::new(RefCell::new(DisplayState::new())),
            system_table: 0,
            boot_services: 0,
            runtime_services: 0,
            gop_interface: 0,
            device_handle: DEVICE_HANDLE,
            efi_app: None,
            images: Vec::new(),
            map_key: 0,
            boot_services_active: true,
            pending_keys: vec![b'1' as u16],
            console: String::new(),
            console_x: 0,
            console_y: 0,
            variables: Vec::new(),
            replay: None,
            replay_instruction_ip: None,
        }
    }

    pub fn reset(&mut self) {
        let replay = self.replay.clone();
        self.state = UefiState::Reset;
        self.images.clear();
        self.map_key = 0;
        self.boot_services_active = true;
        self.pending_keys = vec![b'1' as u16];
        self.console.clear();
        self.console_x = 0;
        self.console_y = 0;
        self.variables.clear();
        self.replay = replay;
        self.replay_instruction_ip = None;
    }

    pub fn attach_replay(&mut self, replay: SharedReplay) {
        self.replay = Some(replay)
    }

    pub fn set_replay_instruction_ip(&mut self, instruction_ip: Option<u64>) {
        self.replay_instruction_ip = instruction_ip
    }

    pub fn set_display(&mut self, display: Rc<RefCell<DisplayState>>) {
        self.display = display;
    }

    pub fn display(&self) -> Rc<RefCell<DisplayState>> {
        self.display.clone()
    }

    pub fn set_memory_size(&mut self, size: usize) {
        self.memory_size = size;
    }

    pub fn memory_size(&self) -> usize {
        self.memory_size
    }

    pub fn set_efi_application(&mut self, image: Vec<u8>) {
        self.efi_app = Some(image);
    }

    pub fn efi_application(&self) -> Option<&[u8]> {
        self.efi_app.as_deref()
    }

    /// Text captured from the firmware console (ConOut.OutputString).
    pub fn console_text(&self) -> &str {
        &self.console
    }

    pub fn system_table_address(&self) -> u64 {
        self.system_table
    }

    pub fn is_boot_services_active(&self) -> bool {
        self.boot_services_active
    }

    // ------------------------------------------------------------------
    // Init: build tables, enter long mode, start the application.
    // ------------------------------------------------------------------

    pub fn init(&mut self, mmu: &mut Mmu, cpu: &mut CpuState) -> Result<(), UefiError> {
        if self.state != UefiState::Reset {
            return Err(UefiError::InvalidState);
        }
        self.build_tables(mmu)?;
        self.enter_long_mode(mmu, cpu)?;

        match self.efi_app.clone() {
            Some(app) => {
                // A small VM can still initialize firmware state even though
                // it cannot place the normal application image aperture.
                if self.memory_size < UEFI_IMAGE_BASE as usize {
                    self.state = UefiState::Initialized;
                    cpu.halted = true;
                    return Ok(())
                }
                let image = self.load_pe(&app)?;
                let mapped = self.map_pe_image(mmu, &image, UEFI_IMAGE_BASE)?;
                let entry = mapped + image.entry_rva as u64;
                let handle = self
                    .register_image(mmu, mapped, entry, image.size_of_image as u64)?;
                self.pending_keys = vec![b'1' as u16];
                self.start_on_cpu(cpu, entry, handle);
                self.state = UefiState::Running;
            }
            None => {
                // No application configured: firmware is initialized but the
                // guest has nothing to execute yet.
                println!(
                    "GhostOS VM: UEFI firmware initialized (SystemTable at 0x{:X})",
                    self.system_table
                );
                println!(
                    "GhostOS VM: no EFI application; pass `--efi <BOOTX64.EFI>` to boot one."
                );
                self.state = UefiState::Initialized;
                cpu.halted = true;
            }
        }
        Ok(())
    }

    /// Install the SystemTable, boot/runtime service tables, console, GOP,
    /// ACPI RSDP and service stubs into guest RAM.
    fn build_tables(&mut self, mmu: &mut Mmu) -> Result<(), UefiError> {
        let base = UEFI_TABLES_BASE;
        self.system_table = base + TBL_SYSTEM_TABLE;
        self.boot_services = base + TBL_BOOT_SERVICES;
        self.runtime_services = base + TBL_RUNTIME_SERVICES;
        self.gop_interface = base + TBL_GOP;

        // Service stubs: `mov eax, id` + `int 0xE0` + `ret` (8 bytes each).
        for id in 0..=SERVICE_RESET_SYSTEM {
            let addr = base + TBL_STUBS + id * 8;
            wmem(mmu, addr, &service_stub(id));
        }
        let unsupported = base + TBL_STUBS + SERVICE_UNSUPPORTED * 8;
        wmem(mmu, base + TBL_RETURN_STUB, &[0xF4]);
        w64(mmu, UEFI_STACK_TOP - 8, base + TBL_RETURN_STUB);
        let stub = |id: u64| base + TBL_STUBS + id * 8;

        // Boot services table (EFI 2.x layout, 0x148 bytes of function slots).
        let bs = self.boot_services;
        write_table_header(mmu, bs, EFI_BOOT_SIGNATURE);
        let mut ptr = |off: u64, v: u64| w64(mmu, bs + off, v);
        ptr(0x38, stub(SERVICE_GET_MEMORY_MAP));
        ptr(0x40, unsupported);
        ptr(0x48, stub(SERVICE_FREE_POOL));
        ptr(0x50, unsupported);
        ptr(0x58, unsupported);
        ptr(0x60, unsupported);
        ptr(0x68, unsupported);
        ptr(0x70, unsupported);
        ptr(0x78, unsupported);
        ptr(0x80, unsupported);
        ptr(0x88, unsupported);
        ptr(0x90, unsupported);
        ptr(0x98, stub(SERVICE_HANDLE_PROTOCOL));
        ptr(0xA0, unsupported);
        ptr(0xA8, unsupported);
        ptr(0xB0, unsupported);
        ptr(0xB8, unsupported);
        ptr(0xC0, unsupported);
        ptr(0xC8, stub(SERVICE_LOAD_IMAGE));
        ptr(0xD0, stub(SERVICE_START_IMAGE));
        ptr(0xD8, unsupported);
        ptr(0xE0, stub(SERVICE_UNLOAD_IMAGE));
        ptr(0xE8, stub(SERVICE_EXIT_BOOT_SERVICES));
        ptr(0xF0, unsupported);
        ptr(0xF8, unsupported);
        ptr(0x100, unsupported);
        ptr(0x108, unsupported);
        ptr(0x110, unsupported);
        ptr(0x118, unsupported);
        ptr(0x120, unsupported);
        ptr(0x128, unsupported);
        ptr(0x130, unsupported);
        ptr(0x138, stub(SERVICE_LOCATE_HANDLE_BUFFER));
        ptr(0x140, stub(SERVICE_LOCATE_PROTOCOL));

        // Runtime services table (EFI 2.x layout, 0x88 bytes).
        let rt = self.runtime_services;
        write_table_header(mmu, rt, EFI_RUNTIME_SIGNATURE);
        let mut rptr = |off: u64, v: u64| w64(mmu, rt + off, v);
        rptr(0x18, stub(SERVICE_GET_TIME));
        rptr(0x20, unsupported);
        rptr(0x28, unsupported);
        rptr(0x30, unsupported);
        rptr(0x38, unsupported);
        rptr(0x40, unsupported);
        rptr(0x48, stub(SERVICE_GET_VARIABLE));
        rptr(0x50, stub(SERVICE_GET_NEXT_VARIABLE));
        rptr(0x58, stub(SERVICE_SET_VARIABLE));
        rptr(0x60, unsupported);
        rptr(0x68, stub(SERVICE_RESET_SYSTEM));
        rptr(0x70, unsupported);
        rptr(0x78, unsupported);
        rptr(0x80, unsupported);

        // ConOut: reset / output_string / clear_screen are real.
        let co = base + TBL_CONSOLE_OUT;
        let mut coptr = |off: u64, v: u64| w64(mmu, co + off, v);
        coptr(0x00, stub(SERVICE_RESET_OUTPUT));
        coptr(0x08, stub(SERVICE_OUTPUT_STRING));
        coptr(0x10, unsupported);
        coptr(0x18, unsupported);
        coptr(0x20, unsupported);
        coptr(0x28, unsupported);
        coptr(0x30, stub(SERVICE_CLEAR_SCREEN));
        coptr(0x38, unsupported);
        coptr(0x40, unsupported);
        coptr(0x48, base + TBL_CONSOLE_MODE);

        // ConIn: reset / read_key_stroke are real.
        let ci = base + TBL_CONSOLE_IN;
        w64(mmu, ci + 0x00, stub(SERVICE_RESET_INPUT));
        w64(mmu, ci + 0x08, stub(SERVICE_READ_KEYSTROKE));
        w64(mmu, ci + 0x10, 0x1122_3344); // wait_for_key event handle

        // Firmware vendor string.
        let vendor = utf16_bytes("GhostOS VM");
        wmem(mmu, base + TBL_FIRMWARE_VENDOR, &vendor);

        // SystemTable.
        let st = self.system_table;
        write_table_header(mmu, st, EFI_SYSTEM_TABLE_SIGNATURE);
        w64(mmu, st + 0x18, base + TBL_FIRMWARE_VENDOR);
        w32(mmu, st + 0x20, 0x0001_0000); // firmware revision
        w64(mmu, st + 0x28, CONSOLE_IN_HANDLE);
        w64(mmu, st + 0x30, base + TBL_CONSOLE_IN);
        w64(mmu, st + 0x38, CONSOLE_OUT_HANDLE);
        w64(mmu, st + 0x40, base + TBL_CONSOLE_OUT);
        w64(mmu, st + 0x48, 0); // stderr handle
        w64(mmu, st + 0x50, 0); // stderr
        w64(mmu, st + 0x58, self.runtime_services);
        w64(mmu, st + 0x60, self.boot_services);
        w64(mmu, st + 0x68, 2); // configuration table entries
        w64(mmu, st + 0x70, base + TBL_CONFIG_TABLE);

        // Configuration table: ACPI 2.0 + ACPI 1.0 point at the RSDP.
        let ct = base + TBL_CONFIG_TABLE;
        wmem(mmu, ct, &ACPI_20_GUID);
        w64(mmu, ct + 16, base + TBL_RSDP);
        wmem(mmu, ct + 24, &ACPI_10_GUID);
        w64(mmu, ct + 40, base + TBL_RSDP);

        // ACPI 2.0 RSDP (36 bytes) with valid checksums.
        build_rsdp(mmu, base + TBL_RSDP);

        // Small virtual disk: one end node device path plus a dummy SFS
        // interface so chainloading can locate a file system.
        wmem(mmu, base + TBL_DEVICE_PATH, &[0x7F, 0xFF, 0x04, 0x00]);
        w64(mmu, base + TBL_FILE_SYSTEM, 0xDEAD_BEEF);
        let file_path = file_device_path("\\EFI\\BOOT\\BOOTX64.EFI");
        wmem(mmu, base + TBL_IMAGE_FILE_PATH, &file_path);

        // GOP protocol (interface + mode + mode-info) backed by the display.
        self.install_gop(mmu, base)?;
        Ok(())
    }

    fn install_gop(&mut self, mmu: &mut Mmu, base: u64) -> Result<(), UefiError> {
        let gop = self.display.borrow().gop();
        let mode = *gop.modes.first().ok_or(UefiError::InitFailed)?;

        let gop_iface = base + TBL_GOP;
        let info = base + TBL_GOP_INFO;
        let mode_struct = base + TBL_GOP_MODE;

        let unsupported = base + TBL_STUBS + SERVICE_UNSUPPORTED * 8;
        w64(mmu, gop_iface + 0x00, unsupported); // QueryMode
        w64(mmu, gop_iface + 0x08, unsupported); // SetMode
        w64(mmu, gop_iface + 0x10, unsupported); // Blt
        w64(mmu, gop_iface + 0x18, mode_struct);

        w32(mmu, mode_struct + 0x00, gop.modes.len() as u32); // MaxMode
        w32(mmu, mode_struct + 0x04, gop.current_mode as u32); // Mode
        w64(mmu, mode_struct + 0x08, info); // Info
        w64(mmu, mode_struct + 0x10, 32); // SizeOfInfo
        w64(mmu, mode_struct + 0x18, gop.framebuffer_base); // FramebufferBase
        w64(mmu, mode_struct + 0x20, gop.framebuffer_size); // FramebufferSize

        w32(mmu, info + 0x00, gop.version); // Version
        w32(mmu, info + 0x04, mode.width);
        w32(mmu, info + 0x08, mode.height);
        w32(mmu, info + 0x0C, mode.pixel_format as u32);
        // PixelInformation[4] at +0x10..+0x20 stays zeroed.
        w32(mmu, info + 0x20, mode.pixels_per_scanline);
        Ok(())
    }

    /// Identity-map all of RAM plus the framebuffer aperture, then take the
    /// CPU to 64-bit long mode with paging on.
    fn enter_long_mode(&mut self, mmu: &mut Mmu, cpu: &mut CpuState) -> Result<(), UefiError> {
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE;
        let page = 4096u64;
        let mut addr = 0u64;
        while addr < self.memory_size as u64 {
            mmu.map_page(addr, addr, flags).map_err(|_| UefiError::LoadFailed)?;
            addr += page;
        }
        // Identity-map the VESA LFB so the GOP framebuffer is usable.
        let mut fb = crate::devices::VESA_LFB_BASE;
        let fb_end = crate::devices::VESA_LFB_BASE + crate::devices::VESA_FB_SIZE as u64;
        while fb < fb_end {
            mmu.map_page(fb, fb, flags).map_err(|_| UefiError::LoadFailed)?;
            fb += page;
        }

        cpu.cr4 |= 1 << 5; // PAE
        cpu.efer |= 1 << 8; // LME
        if cpu.mode == CpuMode::Long64 {
            mmu.set_paging(true, cpu.cr3);
            return Ok(())
        }
        cpu.enter_protected(mmu, 0).map_err(|_| UefiError::InitFailed)?;
        let cr3 = mmu.cr3();
        cpu.enter_long(mmu, cr3).map_err(|_| UefiError::InitFailed)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // EFI application (PE32+) loading
    // ------------------------------------------------------------------

    /// Parse a PE32+ image (the format of EFI applications).
    fn load_pe(&self, bytes: &[u8]) -> Result<PeImage, UefiError> {
        if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
            return Err(UefiError::InvalidImage);
        }
        let read_u16 = |offset: usize| -> Result<u16, UefiError> {
            let end = offset.checked_add(2).ok_or(UefiError::InvalidImage)?;
            let raw = bytes.get(offset..end).ok_or(UefiError::InvalidImage)?;
            Ok(u16::from_le_bytes(
                raw.try_into().map_err(|_| UefiError::InvalidImage)?,
            ))
        };
        let read_u32 = |offset: usize| -> Result<u32, UefiError> {
            let end = offset.checked_add(4).ok_or(UefiError::InvalidImage)?;
            let raw = bytes.get(offset..end).ok_or(UefiError::InvalidImage)?;
            Ok(u32::from_le_bytes(
                raw.try_into().map_err(|_| UefiError::InvalidImage)?,
            ))
        };
        let read_u64 = |offset: usize| -> Result<u64, UefiError> {
            let end = offset.checked_add(8).ok_or(UefiError::InvalidImage)?;
            let raw = bytes.get(offset..end).ok_or(UefiError::InvalidImage)?;
            Ok(u64::from_le_bytes(
                raw.try_into().map_err(|_| UefiError::InvalidImage)?,
            ))
        };
        let e_lfanew = usize::try_from(read_u32(0x3C)?).map_err(|_| UefiError::InvalidImage)?;
        let pe_end = e_lfanew.checked_add(24).ok_or(UefiError::InvalidImage)?;
        if pe_end > bytes.len() || bytes.get(e_lfanew..e_lfanew + 4) != Some(b"PE\0\0") {
            return Err(UefiError::InvalidImage);
        }
        let coff = e_lfanew.checked_add(4).ok_or(UefiError::InvalidImage)?;
        let machine = read_u16(coff)?;
        if machine != 0x8664 {
            return Err(UefiError::Unsupported);
        }
        let num_sections = read_u16(coff.checked_add(2).ok_or(UefiError::InvalidImage)?)? as usize;
        let size_opt =
            read_u16(coff.checked_add(16).ok_or(UefiError::InvalidImage)?)? as usize;
        let opt = coff.checked_add(20).ok_or(UefiError::InvalidImage)?;
        let opt_end = opt.checked_add(size_opt).ok_or(UefiError::InvalidImage)?;
        if size_opt < 112 || opt_end > bytes.len() {
            return Err(UefiError::InvalidImage);
        }
        let magic = read_u16(opt)?;
        if magic != 0x20B {
            // PE32+ only.
            return Err(UefiError::Unsupported);
        }
        let entry_rva = read_u32(opt.checked_add(16).ok_or(UefiError::InvalidImage)?)?;
        let image_base = read_u64(opt.checked_add(24).ok_or(UefiError::InvalidImage)?)?;
        let size_of_image = read_u32(opt.checked_add(56).ok_or(UefiError::InvalidImage)?)?;
        if size_of_image == 0 || entry_rva >= size_of_image {
            return Err(UefiError::InvalidImage);
        }
        let num_dirs = read_u32(opt.checked_add(108).ok_or(UefiError::InvalidImage)?)? as usize;
        let (reloc_rva, reloc_size) = if num_dirs > 5 {
            if size_opt < 112 + 6 * 8 {
                return Err(UefiError::InvalidImage);
            }
            let d = opt.checked_add(112 + 5 * 8).ok_or(UefiError::InvalidImage)?;
            let reloc = (read_u32(d)?, read_u32(d.checked_add(4).ok_or(UefiError::InvalidImage)?)?);
            if reloc.1 > 0
                && reloc.0.checked_add(reloc.1).filter(|end| *end <= size_of_image).is_none()
            {
                return Err(UefiError::InvalidImage);
            }
            reloc
        } else {
            (0, 0)
        };

        let sections_start = opt_end;
        let mut sections = Vec::new();
        for i in 0..num_sections {
            let s = sections_start
                .checked_add(i.checked_mul(40).ok_or(UefiError::InvalidImage)?)
                .ok_or(UefiError::InvalidImage)?;
            let section_end = s.checked_add(40).ok_or(UefiError::InvalidImage)?;
            if section_end > bytes.len() {
                return Err(UefiError::InvalidImage);
            }
            let virtual_address = read_u32(s.checked_add(12).ok_or(UefiError::InvalidImage)?)?;
            let raw_size = read_u32(s.checked_add(16).ok_or(UefiError::InvalidImage)?)? as usize;
            let raw_ptr = read_u32(s.checked_add(20).ok_or(UefiError::InvalidImage)?)? as usize;
            if virtual_address
                .checked_add(raw_size as u32)
                .filter(|end| *end <= size_of_image)
                .is_none()
            {
                return Err(UefiError::InvalidImage);
            }
            let raw_data = if raw_size == 0 {
                Vec::new()
            } else {
                let raw_end = raw_ptr
                    .checked_add(raw_size)
                    .filter(|end| *end <= bytes.len())
                    .ok_or(UefiError::InvalidImage)?;
                bytes[raw_ptr..raw_end].to_vec()
            };
            sections.push(PeSection {
                virtual_address,
                raw_data,
            });
        }
        Ok(PeImage {
            image_base,
            entry_rva,
            size_of_image,
            reloc_rva,
            reloc_size,
            sections,
        })
    }

    /// Map the parsed PE into guest RAM at `requested`, applying base
    /// relocations if the image is loaded somewhere other than its preferred
    /// base. Returns the final load base.
    fn map_pe_image(
        &self,
        mmu: &mut Mmu,
        image: &PeImage,
        requested: u64,
    ) -> Result<u64, UefiError> {
        let end = requested
            .checked_add(image.size_of_image as u64)
            .ok_or(UefiError::OutOfMemory)?;
        if requested % 4096 != 0 || end > self.memory_size as u64 {
            return Err(UefiError::OutOfMemory);
        }
        if self
            .images
            .iter()
            .any(|loaded| requested < loaded.base + loaded.size && loaded.base < end)
        {
            return Err(UefiError::OutOfMemory);
        }

        // Zero-fill the whole image region, then lay sections.
        let zeros = [0u8; 4096];
        let mut zeroed = 0u64;
        while zeroed < image.size_of_image as u64 {
            let count = (image.size_of_image as u64 - zeroed).min(zeros.len() as u64) as usize;
            let address = requested
                .checked_add(zeroed)
                .ok_or(UefiError::OutOfMemory)?;
            mmu.write_phys(address, &zeros[..count])
                .map_err(|_| UefiError::LoadFailed)?;
            zeroed += count as u64;
        }
        for sec in &image.sections {
            if sec.raw_data.is_empty() {
                continue;
            }
            let dst = requested + sec.virtual_address as u64;
            mmu.write_phys(dst, &sec.raw_data)
                .map_err(|_| UefiError::LoadFailed)?;
        }

        let delta = requested as i128 - image.image_base as i128;
        if delta != 0 {
            if image.reloc_size == 0 {
                return Err(UefiError::InvalidImage);
            }
            self.apply_relocations(mmu, requested, delta, image.reloc_rva, image.reloc_size)?;
        }
        Ok(requested)
    }

    fn apply_relocations(
        &self,
        mmu: &mut Mmu,
        base: u64,
        delta: i128,
        reloc_rva: u32,
        reloc_size: u32,
    ) -> Result<(), UefiError> {
        let mut off = 0u64;
        while off + 8 <= reloc_size as u64 {
            let block_addr = base + reloc_rva as u64 + off;
            let page_rva = mmu.read_u32(block_addr).unwrap_or(0) as u64;
            let block_size = mmu.read_u32(block_addr + 4).unwrap_or(0) as u64;
            if block_size < 8 || block_size > reloc_size as u64 - off {
                return Err(UefiError::InvalidImage);
            }
            let count = (block_size - 8) / 2;
            for i in 0..count {
                let entry = mmu.read_u16(block_addr + 8 + i * 2).unwrap_or(0) as u64;
                let ty = (entry >> 12) & 0xF;
                let page_off = entry & 0xFFF;
                let target = base + page_rva + page_off;
                match ty {
                    // DIR64
                    10 => {
                        let v = mmu.read_u64(target).unwrap_or(0);
                        let v = (v as i128).wrapping_add(delta) as u64;
                        let _ = mmu.write_u64(target, v);
                    }
                    // HIGHLOW
                    3 => {
                        let v = mmu.read_u32(target).unwrap_or(0);
                        let v = (v as i64).wrapping_add(delta as i64) as u32;
                        let _ = mmu.write_u32(target, v);
                    }
                    _ => {}
                }
            }
            off += block_size;
        }
        Ok(())
    }

    /// Write an EFI_LOADED_IMAGE_PROTOCOL instance for a new image and return
    /// its handle (the address of the instance).
    fn register_image(
        &mut self,
        mmu: &mut Mmu,
        base: u64,
        entry: u64,
        size: u64,
    ) -> Result<u64, UefiError> {
        let idx = self.images.len();
        if idx >= MAX_IMAGES {
            return Err(UefiError::OutOfMemory);
        }
        let slot = UEFI_TABLES_BASE + TBL_LOADED_IMAGE + (idx as u64) * 0x100;
        let handle = slot;

        let mut b = [0u8; 96];
        b[0..4].copy_from_slice(&0x1000u32.to_le_bytes()); // revision
        b[8..16].copy_from_slice(&0u64.to_le_bytes()); // parent handle
        b[16..24].copy_from_slice(&self.system_table.to_le_bytes());
        b[24..32].copy_from_slice(&self.device_handle.to_le_bytes());
        b[32..40].copy_from_slice(&(UEFI_TABLES_BASE + TBL_IMAGE_FILE_PATH).to_le_bytes());
        b[48..52].copy_from_slice(&0u32.to_le_bytes()); // load options size
        b[56..64].copy_from_slice(&0u64.to_le_bytes()); // load options
        b[64..72].copy_from_slice(&base.to_le_bytes()); // image base
        b[72..80].copy_from_slice(&size.to_le_bytes()); // image size
        b[80..84].copy_from_slice(&0u32.to_le_bytes()); // image code type
        b[84..88].copy_from_slice(&0u32.to_le_bytes()); // image data type
        b[88..96].copy_from_slice(&0u64.to_le_bytes()); // unload fn
        mmu.write_phys(handle, &b).map_err(|_| UefiError::LoadFailed)?;

        self.images.push(UefiImage {
            handle,
            base,
            entry,
            size,
        });
        self.map_key = self.map_key.wrapping_add(1);
        Ok(handle)
    }

    /// Point the CPU at an EFI application entry point per the x64 ABI.
    fn start_on_cpu(&self, cpu: &mut CpuState, entry: u64, handle: u64) {
        cpu.rip = entry;
        cpu.rcx = handle; // ImageHandle
        cpu.rdx = self.system_table; // SystemTable
        cpu.rsp = UEFI_STACK_TOP - 8; // aligns RSP to 8 (mod 16) as after a call
        cpu.rflags = (cpu.rflags | (1 << 9)) | 0x2; // IF + reserved
        cpu.halted = false;
    }

    fn start_child_on_cpu(&self, cpu: &mut CpuState, entry: u64, handle: u64) {
        // StartImage is invoked through a normal call. Preserve that stack
        // so a returning child image resumes at the parent's call site.
        cpu.rip = entry;
        cpu.rcx = handle;
        cpu.rdx = self.system_table;
        cpu.rflags = (cpu.rflags | (1 << 9)) | 0x2;
        cpu.halted = false;
    }

    // ------------------------------------------------------------------
    // Firmware service dispatch (called through `int 0xE0`)
    // ------------------------------------------------------------------

    pub fn dispatch(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        if !self.boot_services_active && is_boot_service(cpu.rax) {
            cpu.rax = EFI_UNSUPPORTED;
            return;
        }
        match cpu.rax {
            SERVICE_OUTPUT_STRING => self.bs_output_string(cpu, mmu),
            SERVICE_CLEAR_SCREEN => self.bs_clear_screen(cpu, mmu),
            SERVICE_RESET_OUTPUT => self.bs_reset_output(cpu),
            SERVICE_READ_KEYSTROKE => self.bs_read_key_stroke(cpu, mmu),
            SERVICE_RESET_INPUT => self.bs_reset_input(cpu),
            SERVICE_GET_MEMORY_MAP => self.bs_get_memory_map(cpu, mmu),
            SERVICE_EXIT_BOOT_SERVICES => self.bs_exit_boot_services(cpu),
            SERVICE_LOCATE_PROTOCOL => self.bs_locate_protocol(cpu, mmu),
            SERVICE_HANDLE_PROTOCOL => self.bs_handle_protocol(cpu, mmu),
            SERVICE_LOCATE_HANDLE_BUFFER => self.bs_locate_handle_buffer(cpu, mmu),
            SERVICE_FREE_POOL => self.bs_free_pool(cpu),
            SERVICE_LOAD_IMAGE => self.bs_load_image(cpu, mmu),
            SERVICE_START_IMAGE => self.bs_start_image(cpu),
            SERVICE_UNLOAD_IMAGE => self.bs_unload_image(cpu),
            SERVICE_GET_VARIABLE => self.rs_get_variable(cpu, mmu),
            SERVICE_SET_VARIABLE => self.rs_set_variable(cpu, mmu),
            SERVICE_GET_NEXT_VARIABLE => self.rs_get_next_variable(cpu, mmu),
            SERVICE_GET_TIME => self.rs_get_time(cpu, mmu),
            SERVICE_RESET_SYSTEM => {
                cpu.halted = true;
                cpu.rax = EFI_SUCCESS;
            }
            _ => cpu.rax = EFI_UNSUPPORTED,
        }
    }

    // ------------------------------------------------------------------
    // Boot services: console
    // ------------------------------------------------------------------

    fn bs_reset_output(&mut self, cpu: &mut CpuState) {
        self.console.clear();
        self.console_x = 0;
        self.console_y = 0;
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_clear_screen(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        // Blank the emulated VGA text buffer through the MMIO path.
        for i in 0..(VGA_COLS * VGA_ROWS) {
            let addr = VGA_TEXT_BASE + (i as u64) * 2;
            let _ = mmu.write_to_addr(addr, b' ' as u64, 1);
            let _ = mmu.write_to_addr(addr + 1, 0x07, 1);
        }
        self.console.clear();
        self.console_x = 0;
        self.console_y = 0;
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_output_string(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let text_ptr = cpu.rdx;
        let chars = read_utf16(mmu, text_ptr, 1024);
        for &c in &chars {
            if c == 0 {
                break;
            }
            let ch = (c & 0xFF) as u8;
            if let Some(ascii) = char::from_u32(c as u32) {
                self.console.push(ascii);
            }
            match ch {
                b'\r' => self.console_x = 0,
                b'\n' => {
                    self.console_x = 0;
                    if self.console_y < (VGA_ROWS as u8).saturating_sub(1) {
                        self.console_y += 1;
                    }
                }
                0x08 => {
                    if self.console_x > 0 {
                        self.console_x -= 1;
                    }
                }
                0x20..=0x7E => {
                    let addr = VGA_TEXT_BASE
                        + ((self.console_y as usize * VGA_COLS + self.console_x as usize) as u64) * 2;
                    let _ = mmu.write_to_addr(addr, ch as u64, 1);
                    let _ = mmu.write_to_addr(addr + 1, 0x07, 1);
                    self.console_x = self.console_x.wrapping_add(1);
                    if self.console_x as usize >= VGA_COLS {
                        self.console_x = 0;
                        if self.console_y < (VGA_ROWS as u8).saturating_sub(1) {
                            self.console_y += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_reset_input(&mut self, cpu: &mut CpuState) {
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_read_key_stroke(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let key_ptr = cpu.rdx;
        if let Some(c) = self.pending_keys.first().copied() {
            self.pending_keys.remove(0);
            if key_ptr != 0 {
                let Some(unicode_ptr) = key_ptr.checked_add(2) else {
                    cpu.rax = EFI_INVALID_PARAMETER;
                    return
                };
                let _ = mmu.write_u16(key_ptr, 0); // scan code
                let _ = mmu.write_u16(unicode_ptr, c); // unicode char
            }
            cpu.rax = EFI_SUCCESS;
        } else {
            cpu.rax = EFI_NOT_READY;
        }
    }

    // ------------------------------------------------------------------
    // Boot services: memory map and handoff
    // ------------------------------------------------------------------

    fn bs_get_memory_map(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let map_size_ptr = cpu.rcx;
        let map_buf = cpu.rdx;
        let map_key_ptr = cpu.r8;
        let desc_size_ptr = cpu.r9;
        let desc_ver_ptr = stack_arg(mmu, cpu, 5);

        if map_size_ptr == 0 || map_key_ptr == 0 || desc_size_ptr == 0 {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }

        let descriptors = self.build_memory_map();
        let needed = (descriptors.len() * MEMORY_DESCRIPTOR_SIZE) as u64;
        let capacity = mmu.read_u64(map_size_ptr).unwrap_or(0);

        if capacity < needed {
            let _ = mmu.write_u64(map_size_ptr, needed);
            if desc_size_ptr != 0 {
                let _ = mmu.write_u64(desc_size_ptr, MEMORY_DESCRIPTOR_SIZE as u64);
            }
            cpu.rax = EFI_BUFFER_TOO_SMALL;
            return;
        }
        if map_buf == 0 {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }

        for (i, desc) in descriptors.iter().enumerate() {
            let Some(addr) = (i as u64)
                .checked_mul(MEMORY_DESCRIPTOR_SIZE as u64)
                .and_then(|offset| map_buf.checked_add(offset))
            else {
                cpu.rax = EFI_INVALID_PARAMETER;
                return
            };
            if mmu.write_phys(addr, desc).is_err() {
                cpu.rax = EFI_INVALID_PARAMETER;
                return
            }
        }
        let _ = mmu.write_u64(map_size_ptr, needed);
        self.map_key = self.map_key.wrapping_add(1);
        let _ = mmu.write_u64(map_key_ptr, self.map_key as u64);
        let _ = mmu.write_u64(desc_size_ptr, MEMORY_DESCRIPTOR_SIZE as u64);
        if desc_ver_ptr != 0 {
            let _ = mmu.write_u32(desc_ver_ptr, 1);
        }
        cpu.rax = EFI_SUCCESS;
    }

    fn build_memory_map(&self) -> Vec<[u8; MEMORY_DESCRIPTOR_SIZE]> {
        let mem = self.memory_size as u64;
        let mut map = Vec::new();
        // Low conventional memory.
        map.push(mem_desc(MEM_TYPE_CONVENTIONAL, 0x000000, 0x0A0000, 0xF));
        // Reserved: VGA/BIOS hole.
        map.push(mem_desc(MEM_TYPE_RESERVED, 0x0A0000, 0x060000, 0x0));
        // UEFI real estate: stack, tables, memory-map buffer.
        map.push(mem_desc(
            MEM_TYPE_LOADER_CODE,
            UEFI_FIRMWARE_START,
            UEFI_IMAGE_BASE - UEFI_FIRMWARE_START,
            0xF,
        ));
        // Loaded EFI applications (or free RAM if none loaded yet).
        let mut cursor = UEFI_IMAGE_BASE;
        let mut images = self.images.iter().collect::<Vec<_>>();
        images.sort_by_key(|image| image.base);
        for image in images {
            if image.base > cursor {
                map.push(mem_desc(
                    MEM_TYPE_CONVENTIONAL,
                    cursor,
                    image.base - cursor,
                    0xF,
                ));
            }
            map.push(mem_desc(
                MEM_TYPE_LOADER_DATA,
                image.base,
                image.size,
                0xF,
            ));
            cursor = cursor.max(image.base.saturating_add(image.size));
        }
        if cursor < mem {
            map.push(mem_desc(MEM_TYPE_CONVENTIONAL, cursor, mem - cursor, 0xF));
        }
        map
    }

    fn bs_exit_boot_services(&mut self, cpu: &mut CpuState) {
        if !self.boot_services_active
            || !self.images.iter().any(|image| image.handle == cpu.rcx)
        {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }
        let key = cpu.rdx;
        if key == self.map_key as u64 {
            self.boot_services_active = false;
            self.state = UefiState::ExitBootServices;
            cpu.rax = EFI_SUCCESS;
        } else {
            cpu.rax = EFI_NOT_READY;
        }
    }

    // ------------------------------------------------------------------
    // Boot services: protocol handling
    // ------------------------------------------------------------------

    fn bs_locate_protocol(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let protocol = cpu.rcx;
        let _registration = cpu.rdx;
        let interface_ptr = cpu.r8;
        let guid = read_guid(mmu, protocol);

        let interface = match guid {
            Some(g) if g == GOP_GUID => Some(self.gop_interface),
            Some(g) if g == FILE_SYSTEM_GUID => Some(UEFI_TABLES_BASE + TBL_FILE_SYSTEM),
            _ => None,
        };

        match interface {
            Some(iface) => {
                let _ = mmu.write_u64(interface_ptr, iface);
                cpu.rax = EFI_SUCCESS;
            }
            None => cpu.rax = EFI_NOT_FOUND,
        }
    }

    fn bs_handle_protocol(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let handle = cpu.rcx;
        let protocol = cpu.rdx;
        let interface_ptr = cpu.r8;
        let guid = read_guid(mmu, protocol);

        let interface = if self.images.iter().any(|i| i.handle == handle) {
            match guid {
                Some(g) if g == LOADED_IMAGE_GUID => Some(handle),
                _ => None,
            }
        } else if handle == self.device_handle {
            match guid {
                Some(g) if g == DEVICE_PATH_GUID => Some(UEFI_TABLES_BASE + TBL_DEVICE_PATH),
                Some(g) if g == FILE_SYSTEM_GUID => Some(UEFI_TABLES_BASE + TBL_FILE_SYSTEM),
                _ => None,
            }
        } else {
            None
        };

        match interface {
            Some(iface) => {
                let _ = mmu.write_u64(interface_ptr, iface);
                cpu.rax = EFI_SUCCESS;
            }
            None => cpu.rax = EFI_NOT_FOUND,
        }
    }

    fn bs_locate_handle_buffer(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let search_type = cpu.rcx;
        let protocol = cpu.rdx;
        let _search_key = cpu.r8;
        let count_ptr = cpu.r9;
        let buffer_ptr = stack_arg(mmu, cpu, 5);
        let guid = read_guid(mmu, protocol);

        // Only BY_PROTOCOL (2) against the simple file system is supported.
        if search_type != 2 || guid != Some(FILE_SYSTEM_GUID) {
            cpu.rax = EFI_NOT_FOUND;
            return;
        }
        if count_ptr == 0 || buffer_ptr == 0 {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }
        let handles = UEFI_TABLES_BASE + TBL_HANDLE_BUFFER;
        let _ = mmu.write_u64(handles, self.device_handle);
        let _ = mmu.write_u64(count_ptr, 1);
        let _ = mmu.write_u64(buffer_ptr, handles);
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_free_pool(&mut self, cpu: &mut CpuState) {
        // All handle buffers live in fixed firmware slots; nothing to free.
        cpu.rax = EFI_SUCCESS;
    }

    // ------------------------------------------------------------------
    // Boot services: image loading
    // ------------------------------------------------------------------

    fn bs_load_image(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let _boot_policy = cpu.rcx;
        let _parent = cpu.rdx;
        let device_path = cpu.r8;
        let _source_buffer = cpu.r9;
        let _source_size = stack_arg(mmu, cpu, 5);
        let image_handle_ptr = stack_arg(mmu, cpu, 6);

        let Some(app) = self.efi_app.clone() else {
            cpu.rax = EFI_NOT_FOUND;
            return;
        };

        // Ignore the exact path — the VM exposes a single boot volume whose
        // EFI application is the configured image. Chainload of the GhostOS
        // boot manager therefore always succeeds.
        let _ = device_path_filename(mmu, device_path);

        let image = match self.load_pe(&app) {
            Ok(i) => i,
            Err(_) => {
                cpu.rax = EFI_LOAD_ERROR;
                return;
            }
        };

        // Place chainloaded images at a distinct base so the running parent
        // is not overwritten; fall back if RAM is tight.
        let requested = if self.images.is_empty() {
            UEFI_IMAGE_BASE
        } else {
            UEFI_CHILD_IMAGE_BASE
        };
        let mapped = match self.map_pe_image(mmu, &image, requested) {
            Ok(b) => b,
            Err(_) => match self.map_pe_image(mmu, &image, UEFI_IMAGE_BASE) {
                Ok(b) => b,
                Err(_) => {
                    cpu.rax = EFI_OUT_OF_RESOURCES;
                    return;
                }
            },
        };
        let entry = mapped + image.entry_rva as u64;
        let size = image.size_of_image as u64;

        let handle = match self.register_image(mmu, mapped, entry, size) {
            Ok(h) => h,
            Err(_) => {
                cpu.rax = EFI_OUT_OF_RESOURCES;
                return;
            }
        };
        if image_handle_ptr != 0 {
            let _ = mmu.write_u64(image_handle_ptr, handle);
        }
        cpu.rax = EFI_SUCCESS;
    }

    fn bs_start_image(&mut self, cpu: &mut CpuState) {
        let handle = cpu.rcx;
        if let Some(image) = self.images.iter().find(|i| i.handle == handle) {
            self.pending_keys = vec![b'1' as u16];
            self.start_child_on_cpu(cpu, image.entry, image.handle);
            self.state = UefiState::Running;
            cpu.rax = EFI_SUCCESS;
        } else {
            cpu.rax = EFI_NOT_FOUND;
        }
    }

    fn bs_unload_image(&mut self, cpu: &mut CpuState) {
        if let Some(index) = self.images.iter().position(|image| image.handle == cpu.rcx) {
            self.images.remove(index);
            self.map_key = self.map_key.wrapping_add(1);
            cpu.rax = EFI_SUCCESS;
        } else {
            cpu.rax = EFI_NOT_FOUND;
        }
    }

    // ------------------------------------------------------------------
    // Runtime services
    // ------------------------------------------------------------------

    fn rs_get_time(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let time_ptr = cpu.rcx;
        if time_ptr == 0 {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now = self
            .replay
            .as_ref()
            .map(|replay| {
                replay
                    .borrow_mut()
                    .instruction_input(
                        self.replay_instruction_ip.unwrap_or(0),
                        0x5545_4649_4745_5454,
                        8,
                        now,
                    )
                    .unwrap_or(now)
            })
            .unwrap_or(now);
        let days = now / 86400;
        let secs = now % 86400;
        let (year, month, day) = civil_date_from_days(days as i64);
        let mut b = [0u8; 16];
        b[0..2].copy_from_slice(&year.to_le_bytes());
        b[2] = month as u8;
        b[3] = day as u8;
        b[4] = ((secs / 3600) % 24) as u8;
        b[5] = ((secs / 60) % 60) as u8;
        b[6] = (secs % 60) as u8;
        // Nanosecond (8), TimeZone=EFI_UNSPECIFIED_TIMEZONE (12), Daylight=0.
        b[12..14].copy_from_slice(&0x07FFu16.to_le_bytes());
        let _ = mmu.write_phys(time_ptr, &b);
        cpu.rax = EFI_SUCCESS;
    }

    fn rs_get_variable(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let name_ptr = cpu.rcx;
        let guid_ptr = cpu.rdx;
        let attributes_ptr = cpu.r8;
        let data_size_ptr = cpu.r9;
        let data_ptr = stack_arg(mmu, cpu, 5);

        let Some(guid) = read_guid(mmu, guid_ptr) else {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        };
        let name = read_utf16(mmu, name_ptr, 1024);
        let found = self
            .variables
            .iter()
            .find(|(n, g, _, _)| *n == name && *g == guid)
            .cloned();

        match found {
            Some((_, _, data, attrs)) => {
                let capacity = mmu.read_u64(data_size_ptr).unwrap_or(0);
                if capacity < data.len() as u64 {
                    let _ = mmu.write_u64(data_size_ptr, data.len() as u64);
                    cpu.rax = EFI_BUFFER_TOO_SMALL;
                    return;
                }
                let _ = mmu.write_u64(data_size_ptr, data.len() as u64);
                let _ = mmu.write_u32(attributes_ptr, attrs);
                if !data.is_empty() && data_ptr != 0 {
                    let _ = mmu.write_phys(data_ptr, &data);
                }
                cpu.rax = EFI_SUCCESS;
            }
            None => {
                let _ = mmu.write_u64(data_size_ptr, 0);
                cpu.rax = EFI_NOT_FOUND;
            }
        }
    }

    fn rs_set_variable(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let name_ptr = cpu.rcx;
        let guid_ptr = cpu.rdx;
        let attributes = cpu.r8 as u32;
        let data_size = stack_arg(mmu, cpu, 5);
        let data_ptr = stack_arg(mmu, cpu, 6);

        let Some(guid) = read_guid(mmu, guid_ptr) else {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        };
        let name = read_utf16(mmu, name_ptr, 1024);
        let mut data = Vec::new();
        if data_size > 0 && data_ptr != 0 {
            let Ok(data_len) = usize::try_from(data_size) else {
                cpu.rax = EFI_INVALID_PARAMETER;
                return
            };
            data = mmu.read_phys(data_ptr, data_len).unwrap_or_default();
        }

        if data_size == 0 {
            self.variables
                .retain(|(n, g, _, _)| *n != name || *g != guid);
        } else if let Some(slot) = self
            .variables
            .iter_mut()
            .find(|(n, g, _, _)| *n == name && *g == guid)
        {
            slot.2 = data;
            slot.3 = attributes;
        } else {
            self.variables.push((name, guid, data, attributes));
        }
        cpu.rax = EFI_SUCCESS;
    }

    fn rs_get_next_variable(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let size_ptr = cpu.rcx;
        let name_buf = cpu.rdx;
        let guid_ptr = cpu.r8;

        if size_ptr == 0 || name_buf == 0 || guid_ptr == 0 {
            cpu.rax = EFI_INVALID_PARAMETER;
            return;
        }
        let current_name = read_utf16(mmu, name_buf, 1024);
        let current_guid = read_guid(mmu, guid_ptr).unwrap_or([0; 16]);
        let next = if current_name.is_empty() {
            0
        } else if let Some(index) = self
            .variables
            .iter()
            .position(|(name, guid, _, _)| *name == current_name && *guid == current_guid)
        {
            index + 1
        } else {
            cpu.rax = EFI_NOT_FOUND;
            return;
        };
        let Some((name, guid, _, _attrs)) = self.variables.get(next) else {
            cpu.rax = EFI_NOT_FOUND;
            return;
        };
        let needed = (name.len() as u64 + 1) * 2; // includes NUL terminator
        let capacity = mmu.read_u64(size_ptr).unwrap_or(0);
        if capacity < needed {
            let _ = mmu.write_u64(size_ptr, needed);
            cpu.rax = EFI_BUFFER_TOO_SMALL;
            return;
        }
        let mut bytes = Vec::with_capacity(needed as usize);
        for &c in name {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        let _ = mmu.write_phys(name_buf, &bytes);
        let _ = mmu.write_phys(guid_ptr, guid);
        let _ = mmu.write_u64(size_ptr, needed);
        cpu.rax = EFI_SUCCESS;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_boot_service(id: u64) -> bool {
    matches!(
        id,
        SERVICE_OUTPUT_STRING
            | SERVICE_CLEAR_SCREEN
            | SERVICE_RESET_OUTPUT
            | SERVICE_READ_KEYSTROKE
            | SERVICE_RESET_INPUT
            | SERVICE_GET_MEMORY_MAP
            | SERVICE_EXIT_BOOT_SERVICES
            | SERVICE_LOCATE_PROTOCOL
            | SERVICE_HANDLE_PROTOCOL
            | SERVICE_LOCATE_HANDLE_BUFFER
            | SERVICE_FREE_POOL
            | SERVICE_LOAD_IMAGE
            | SERVICE_START_IMAGE
            | SERVICE_UNLOAD_IMAGE
    )
}

fn civil_date_from_days(days_since_epoch: i64) -> (u16, u32, u32) {
    // Howard Hinnant's proleptic Gregorian conversion. UEFI reports UTC.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as u16, month as u32, day as u32)
}

fn file_device_path(path: &str) -> Vec<u8> {
    let path_bytes = utf16_bytes(path);
    let node_length = 4 + path_bytes.len();
    let mut out = Vec::with_capacity(node_length + 4);
    out.push(0x04);
    out.push(0x04);
    out.extend_from_slice(&(node_length as u16).to_le_bytes());
    out.extend_from_slice(&path_bytes);
    out.extend_from_slice(&[0x7F, 0xFF, 0x04, 0x00]);
    out
}

/// Emit the 8-byte service stub: `mov eax, imm32; int 0xE0; ret`.
fn service_stub(id: u64) -> [u8; 8] {
    let mut s = [0u8; 8];
    s[0] = 0xB8;
    s[1..5].copy_from_slice(&(id as u32).to_le_bytes());
    s[5] = 0xCD;
    s[6] = UEFI_CALL_VECTOR;
    s[7] = 0xC3;
    s
}

/// Write a 24-byte EFI table header (signature/revision/header-size).
fn write_table_header(mmu: &mut Mmu, addr: u64, signature: u64) {
    let mut hdr = [0u8; EFI_TABLE_HEADER_SIZE as usize];
    hdr[0..8].copy_from_slice(&signature.to_le_bytes());
    hdr[8..12].copy_from_slice(&EFI_TABLE_REVISION.to_le_bytes());
    hdr[12..16].copy_from_slice(&EFI_TABLE_HEADER_SIZE.to_le_bytes());
    wmem(mmu, addr, &hdr);
}

/// ACPI 2.0 RSDP: 36 bytes with `RSD PTR ` signature and valid checksums.
fn build_rsdp(mmu: &mut Mmu, addr: u64) {
    let mut b = [0u8; 36];
    b[0..8].copy_from_slice(b"RSD PTR ");
    b[8] = 2; // ACPI version 2.0
    b[15] = 36; // Length includes the extended portion
    // Revision 2 RSDP: RSDT/XSDT addresses are at offset 16 (u32) and 24 (u64).
    // We leave them zero; OSes walk the UEFI config table instead.
    let sum1: u8 = b[0..20].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    b[9] = (0u8.wrapping_sub(sum1)) & 0xFF; // checksum of first 20 bytes
    let sum2: u8 = b.iter().fold(0u8, |a, &x| a.wrapping_add(x));
    b[32] = (0u8.wrapping_sub(sum2)) & 0xFF; // extended checksum
    wmem(mmu, addr, &b);
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((s.len() + 1) * 2);
    for c in s.encode_utf16() {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn wmem(mmu: &mut Mmu, addr: u64, bytes: &[u8]) {
    let _ = mmu.write_phys(addr, bytes);
}

fn w64(mmu: &mut Mmu, addr: u64, v: u64) {
    let _ = mmu.write_u64(addr, v);
}

fn w32(mmu: &mut Mmu, addr: u64, v: u32) {
    let _ = mmu.write_u32(addr, v);
}

/// EFI x64 ABI: the 5th argument is at `RSP + 0x28` at stub entry.
fn stack_arg(mmu: &Mmu, cpu: &CpuState, idx: u64) -> u64 {
    if idx < 5 {
        match idx {
            1 => cpu.rcx,
            2 => cpu.rdx,
            3 => cpu.r8,
            4 => cpu.r9,
            _ => 0,
        }
    } else {
        let Some(addr) = (idx - 5)
            .checked_mul(8)
            .and_then(|offset| 0x28u64.checked_add(offset))
            .and_then(|offset| cpu.rsp.checked_add(offset))
        else {
            return 0
        };
        mmu.read_u64(addr).unwrap_or(0)
    }
}

fn read_guid(mmu: &Mmu, ptr: u64) -> Option<[u8; 16]> {
    if ptr == 0 {
        return None;
    }
    let bytes = mmu.read_phys(ptr, 16).ok()?;
    bytes.try_into().ok()
}

fn read_utf16(mmu: &Mmu, ptr: u64, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    if ptr == 0 {
        return out;
    }
    for i in 0..max {
        let Some(addr) = (i as u64)
            .checked_mul(2)
            .and_then(|offset| ptr.checked_add(offset))
        else {
            break
        };
        let c = mmu.read_u16(addr).unwrap_or(0);
        if c == 0 {
            break;
        }
        out.push(c);
    }
    out
}

/// Extract the Media FilePath node from an EFI device path. Used only for
/// validation; the VM hands back the same application regardless.
fn device_path_filename(mmu: &Mmu, path_ptr: u64) -> Vec<u16> {
    let mut out = Vec::new();
    if path_ptr == 0 {
        return out;
    }
    let mut off = 0u64;
    for _ in 0..16 {
        let Some(node_addr) = path_ptr.checked_add(off) else {
            break
        };
        let hdr = mmu.read_phys(node_addr, 4).unwrap_or_default();
        if hdr.len() < 4 {
            break;
        }
        let node_type = hdr[0];
        let sub_type = hdr[1];
        let len = u16::from_le_bytes([hdr[2], hdr[3]]) as u64;
        if len < 4 {
            break;
        }
        if node_type == 0x7F && sub_type == 0xFF {
            break;
        }
        if node_type == 0x04 && sub_type == 0x04 {
            // Media FilePath: UTF-16 file name follows the 4-byte header.
            let name_bytes = (len - 4) as usize;
            for i in 0..name_bytes / 2 {
                let Some(addr) = (i as u64)
                    .checked_mul(2)
                    .and_then(|index| off.checked_add(4 + index))
                    .and_then(|index| path_ptr.checked_add(index))
                else {
                    break
                };
                let c = mmu.read_u16(addr).unwrap_or(0);
                if c == 0 {
                    break;
                }
                out.push(c);
            }
            break;
        }
        let Some(next) = off.checked_add(len) else {
            break
        };
        off = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuState;

    fn mem_mmu() -> Mmu {
        Mmu::new(32 * 1024 * 1024)
    }

    #[test]
    fn service_stub_encodes_call_vector() {
        let stub = service_stub(5);
        assert_eq!(stub[0], 0xB8);
        assert_eq!(u32::from_le_bytes(stub[1..5].try_into().unwrap()), 5);
        assert_eq!(stub[5], 0xCD);
        assert_eq!(stub[6], UEFI_CALL_VECTOR);
        assert_eq!(stub[7], 0xC3);
    }

    #[test]
    fn init_builds_system_table() {
        let mut ctx = UefiContext::new();
        ctx.set_memory_size(64 * 1024 * 1024);
        let mut mmu = Mmu::new(64 * 1024 * 1024);
        let mut cpu = CpuState::default();
        ctx.init(&mut mmu, &mut cpu).unwrap();
        assert_eq!(ctx.state, UefiState::Initialized);
        assert!(cpu.halted);
        let st = ctx.system_table_address();
        assert_ne!(st, 0);
        let sig = mmu.read_u64(st).unwrap();
        assert_eq!(sig, EFI_SYSTEM_TABLE_SIGNATURE);
        // Boot services pointer inside the SystemTable.
        let bs = mmu.read_u64(st + 0x60).unwrap();
        let bs_sig = mmu.read_u64(bs).unwrap();
        assert_eq!(bs_sig, EFI_BOOT_SIGNATURE);
    }

    #[test]
    fn memory_map_has_expected_regions() {
        let ctx = UefiContext::new();
        let map = ctx.build_memory_map();
        assert!(map.len() >= 3);
        // First region is conventional memory starting at 0.
        let first = map[0];
        assert_eq!(u32::from_le_bytes(first[0..4].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(first[8..16].try_into().unwrap()), 0);
    }

    #[test]
    fn relocations_do_not_crash() {
        let mut mmu = mem_mmu();
        let ctx = UefiContext::new();
        // Simulate a tiny relocation block with one DIR64 entry.
        let base = 0x0080_0000u64;
        let reloc_rva = 0x2000u32; // block lives at base+0x2000
        let block = base + reloc_rva as u64;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1000u32.to_le_bytes()); // page RVA (target)
        bytes.extend_from_slice(&12u32.to_le_bytes()); // block size
        bytes.extend_from_slice(&((10u16 << 12) | 0x000).to_le_bytes()); // DIR64
        mmu.write_phys(block, &bytes).unwrap();
        // The relocation target (page RVA + page offset) is at base+0x1000,
        // distinct from the block so the header is not overwritten.
        mmu.write_u64(base + 0x1000, 0x1234_5678).unwrap();
        ctx.apply_relocations(&mut mmu, base, 0x1000, reloc_rva, 12)
            .unwrap();
        assert_eq!(mmu.read_u64(base + 0x1000).unwrap(), 0x1234_6678);
    }

    #[test]
    fn dispatch_unsupported_service_returns_unsupported() {
        let mut ctx = UefiContext::new();
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        cpu.rax = SERVICE_UNSUPPORTED;
        ctx.dispatch(&mut cpu, &mut mmu);
        assert_eq!(cpu.rax, EFI_UNSUPPORTED);
    }

    #[test]
    fn utf16_roundtrip() {
        let bytes = utf16_bytes("AB");
        // 'A' (0x41), 'B' (0x42), then NUL terminator.
        assert_eq!(bytes, vec![0x41, 0x00, 0x42, 0x00, 0x00, 0x00]);
    }
}
