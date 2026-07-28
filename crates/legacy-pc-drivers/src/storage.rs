use core::ptr::{NonNull, read_volatile, write_volatile};
use synos_status::{IntoStatus, Severity, Status, facility};

use crate::pci::PciDevice;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageKind {
    Ahci,
    Nvme,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageController {
    pub kind: StorageKind,
    pub registers: u64,
    pub interrupt_line: u8,
}

impl StorageController {
    pub fn from_pci(device: &PciDevice) -> Option<Self> {
        let (kind, bar) = if device.is_ahci() {
            (StorageKind::Ahci, device.bars[5])
        } else if device.is_nvme() {
            (StorageKind::Nvme, device.bars[0])
        } else {
            return None
        };

        Some(Self {
            kind,
            registers: bar.memory_address()?,
            interrupt_line: device.interrupt_line,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverError {
    InvalidRegisterBase,
    InvalidQueue,
    InvalidRequest,
    NoDevice,
    TimedOut,
    ControllerFault,
}

impl IntoStatus for DriverError {
    fn status(self) -> Status {
        let (severity, code) = match self {
            Self::InvalidRegisterBase => (Severity::Error, 10),
            Self::InvalidQueue => (Severity::Error, 11),
            Self::InvalidRequest => (Severity::Error, 12),
            Self::NoDevice => return Status::NOT_FOUND,
            Self::TimedOut => (Severity::Error, 13),
            Self::ControllerFault => (Severity::Fatal, 14),
        };
        Status::new(severity, facility::DRIVER, code, 0).expect("valid storage status")
    }
}

const AHCI_GHC_ENABLE: u32 = 1 << 31;
const AHCI_PORT_START: u32 = 1;
const AHCI_FIS_RECEIVE: u32 = 1 << 4;
const AHCI_COMMAND_RUNNING: u32 = 1 << 15;
const AHCI_FIS_RUNNING: u32 = 1 << 14;
const AHCI_TASK_BUSY: u32 = 0x88;

#[repr(C)]
struct AhciHostRegisters {
    capabilities: u32,
    global_control: u32,
    interrupt_status: u32,
    ports_implemented: u32,
}

#[repr(C)]
struct AhciPortRegisters {
    command_list_base: u64,
    fis_base: u64,
    interrupt_status: u32,
    interrupt_enable: u32,
    command: u32,
    reserved: u32,
    task_file_data: u32,
    signature: u32,
    sata_status: u32,
    sata_control: u32,
    sata_error: u32,
    sata_active: u32,
    command_issue: u32,
}

pub struct AhciController {
    registers: NonNull<u8>,
}

impl AhciController {
    /// # Safety
    /// `registers` must be a writable mapping of the controller's complete
    /// AHCI BAR, owned exclusively by this driver.
    pub unsafe fn new(registers: usize) -> Result<Self, DriverError> {
        Ok(Self {
            registers: NonNull::new(registers as *mut u8)
                .ok_or(DriverError::InvalidRegisterBase)?,
        })
    }

    pub fn enable(&mut self) {
        unsafe {
            let host = self.registers.as_ptr().cast::<AhciHostRegisters>();
            let control = read_volatile(&(*host).global_control);
            write_volatile(&mut (*host).global_control, control | AHCI_GHC_ENABLE)
        }
    }

    pub fn ports_implemented(&self) -> u32 {
        unsafe {
            let host = self.registers.as_ptr().cast::<AhciHostRegisters>();
            read_volatile(&(*host).ports_implemented)
        }
    }

    pub fn port(&mut self, index: u8) -> Option<AhciPort> {
        if index >= 32 || self.ports_implemented() & (1 << index) == 0 {
            return None
        }

        let address = unsafe {
            self.registers
                .as_ptr()
                .add(0x100 + index as usize * 0x80)
                .cast::<AhciPortRegisters>()
        };
        Some(AhciPort {
            registers: unsafe { NonNull::new_unchecked(address) },
        })
    }
}

pub struct AhciPort {
    registers: NonNull<AhciPortRegisters>,
}

impl AhciPort {
    pub fn has_sata_device(&self) -> bool {
        let status = unsafe { read_volatile(&self.registers.as_ref().sata_status) };
        status & 0x0f == 3 && (status >> 8) & 0x0f == 1
    }

    pub fn stop(&mut self, spin_limit: usize) -> Result<(), DriverError> {
        unsafe {
            let registers = self.registers.as_mut();
            let command = read_volatile(&registers.command);
            write_volatile(
                &mut registers.command,
                command & !(AHCI_PORT_START | AHCI_FIS_RECEIVE),
            );
            wait_clear(
                &registers.command,
                AHCI_COMMAND_RUNNING | AHCI_FIS_RUNNING,
                spin_limit,
            )
        }
    }

    /// Installs caller-owned DMA areas. The command list must be 1 KiB aligned
    /// and the received-FIS area must be 256-byte aligned.
    pub fn configure(
        &mut self,
        command_list_physical: u64,
        received_fis_physical: u64,
    ) -> Result<(), DriverError> {
        if command_list_physical & 0x3ff != 0 || received_fis_physical & 0xff != 0 {
            return Err(DriverError::InvalidQueue)
        }

        unsafe {
            let registers = self.registers.as_mut();
            write_volatile(
                &mut registers.command_list_base,
                command_list_physical,
            );
            write_volatile(&mut registers.fis_base, received_fis_physical);
            write_volatile(&mut registers.sata_error, u32::MAX);
            write_volatile(&mut registers.interrupt_status, u32::MAX)
        }
        Ok(())
    }

    pub fn start(&mut self) {
        unsafe {
            let registers = self.registers.as_mut();
            let command = read_volatile(&registers.command);
            write_volatile(
                &mut registers.command,
                command | AHCI_FIS_RECEIVE | AHCI_PORT_START,
            )
        }
    }

    pub fn issue(&mut self, slot: u8, spin_limit: usize) -> Result<(), DriverError> {
        if slot >= 32 {
            return Err(DriverError::InvalidRequest)
        }
        let mask = 1 << slot;

        unsafe {
            let registers = self.registers.as_mut();
            let mut spins = spin_limit;
            while read_volatile(&registers.task_file_data) & AHCI_TASK_BUSY != 0 {
                if spins == 0 {
                    return Err(DriverError::TimedOut)
                }
                spins -= 1;
                core::hint::spin_loop()
            }

            write_volatile(&mut registers.command_issue, mask);
            wait_clear(&registers.command_issue, mask, spin_limit)?;
            if read_volatile(&registers.interrupt_status) & (1 << 30) != 0 {
                return Err(DriverError::ControllerFault)
            }
        }
        Ok(())
    }
}

fn wait_clear(
    register: *const u32,
    mask: u32,
    mut spins: usize,
) -> Result<(), DriverError> {
    while unsafe { read_volatile(register) } & mask != 0 {
        if spins == 0 {
            return Err(DriverError::TimedOut)
        }
        spins -= 1;
        core::hint::spin_loop()
    }
    Ok(())
}

#[repr(C, align(32))]
#[derive(Clone, Copy)]
pub struct AhciCommandHeader {
    pub flags: u16,
    pub prdt_length: u16,
    pub transferred_bytes: u32,
    pub command_table_base: u64,
    pub reserved: [u32; 4],
}

impl AhciCommandHeader {
    pub const EMPTY: Self = Self {
        flags: 0,
        prdt_length: 0,
        transferred_bytes: 0,
        command_table_base: 0,
        reserved: [0; 4],
    };
}

#[repr(C, align(1024))]
pub struct AhciCommandList {
    pub entries: [AhciCommandHeader; 32],
}

impl AhciCommandList {
    pub const fn new() -> Self {
        Self {
            entries: [AhciCommandHeader::EMPTY; 32],
        }
    }
}

impl Default for AhciCommandList {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AhciPrdtEntry {
    pub data_base: u64,
    pub reserved: u32,
    pub byte_count_and_flags: u32,
}

#[repr(C, align(128))]
pub struct AhciCommandTable {
    pub command_fis: [u8; 64],
    pub atapi_command: [u8; 16],
    pub reserved: [u8; 48],
    pub prdt: [AhciPrdtEntry; 1],
}

impl AhciCommandTable {
    pub const fn new() -> Self {
        Self {
            command_fis: [0; 64],
            atapi_command: [0; 16],
            reserved: [0; 48],
            prdt: [AhciPrdtEntry {
                data_base: 0,
                reserved: 0,
                byte_count_and_flags: 0,
            }],
        }
    }

    pub fn prepare_dma(
        &mut self,
        write: bool,
        lba: u64,
        sectors: u16,
        buffer_physical: u64,
        byte_count: u32,
    ) -> Result<(), DriverError> {
        if sectors == 0 || byte_count == 0 || byte_count > 4 * 1024 * 1024 {
            return Err(DriverError::InvalidRequest)
        }

        self.command_fis.fill(0);
        self.command_fis[0] = 0x27;
        self.command_fis[1] = 1 << 7;
        self.command_fis[2] = if write { 0x35 } else { 0x25 };
        self.command_fis[4] = lba as u8;
        self.command_fis[5] = (lba >> 8) as u8;
        self.command_fis[6] = (lba >> 16) as u8;
        self.command_fis[7] = 1 << 6;
        self.command_fis[8] = (lba >> 24) as u8;
        self.command_fis[9] = (lba >> 32) as u8;
        self.command_fis[10] = (lba >> 40) as u8;
        self.command_fis[12] = sectors as u8;
        self.command_fis[13] = (sectors >> 8) as u8;
        self.prdt[0] = AhciPrdtEntry {
            data_base: buffer_physical,
            reserved: 0,
            byte_count_and_flags: (byte_count - 1) | (1 << 31),
        };
        Ok(())
    }

    pub fn prepare_header(
        &self,
        header: &mut AhciCommandHeader,
        table_physical: u64,
        write: bool,
    ) {
        *header = AhciCommandHeader {
            flags: 5 | if write { 1 << 6 } else { 0 },
            prdt_length: 1,
            transferred_bytes: 0,
            command_table_base: table_physical,
            reserved: [0; 4],
        }
    }
}

impl Default for AhciCommandTable {
    fn default() -> Self {
        Self::new()
    }
}

const NVME_CC_ENABLE: u32 = 1;
const NVME_CSTS_READY: u32 = 1;
const NVME_CSTS_FATAL: u32 = 1 << 1;

#[repr(C)]
struct NvmeRegisters {
    capabilities: u64,
    version: u32,
    interrupt_mask_set: u32,
    interrupt_mask_clear: u32,
    controller_config: u32,
    reserved0: u32,
    controller_status: u32,
    subsystem_reset: u32,
    admin_queue_attributes: u32,
    admin_submission_queue: u64,
    admin_completion_queue: u64,
}

pub struct NvmeController {
    registers: NonNull<NvmeRegisters>,
}

impl NvmeController {
    /// # Safety
    /// `registers` must be a writable, exclusive mapping of the NVMe BAR.
    pub unsafe fn new(registers: usize) -> Result<Self, DriverError> {
        Ok(Self {
            registers: NonNull::new(registers as *mut NvmeRegisters)
                .ok_or(DriverError::InvalidRegisterBase)?,
        })
    }

    pub fn doorbell_stride(&self) -> usize {
        let capabilities = unsafe { read_volatile(&self.registers.as_ref().capabilities) };
        4 << ((capabilities >> 32) & 0x0f)
    }

    pub fn reset(&mut self, spin_limit: usize) -> Result<(), DriverError> {
        unsafe {
            let registers = self.registers.as_mut();
            let config = read_volatile(&registers.controller_config);
            write_volatile(
                &mut registers.controller_config,
                config & !NVME_CC_ENABLE,
            );
            wait_clear(&registers.controller_status, NVME_CSTS_READY, spin_limit)
        }
    }

    pub fn configure_admin_queue(
        &mut self,
        depth: u16,
        submission_physical: u64,
        completion_physical: u64,
    ) -> Result<(), DriverError> {
        if depth < 2
            || submission_physical & 0xfff != 0
            || completion_physical & 0xfff != 0
        {
            return Err(DriverError::InvalidQueue)
        }

        unsafe {
            let registers = self.registers.as_mut();
            let size = (depth as u32 - 1) & 0xfff;
            write_volatile(&mut registers.admin_queue_attributes, size | size << 16);
            write_volatile(
                &mut registers.admin_submission_queue,
                submission_physical,
            );
            write_volatile(
                &mut registers.admin_completion_queue,
                completion_physical,
            )
        }
        Ok(())
    }

    pub fn enable(&mut self, spin_limit: usize) -> Result<(), DriverError> {
        unsafe {
            let registers = self.registers.as_mut();
            let config = 6 << 16 | 4 << 20 | NVME_CC_ENABLE;
            write_volatile(&mut registers.controller_config, config);

            let mut spins = spin_limit;
            loop {
                let status = read_volatile(&registers.controller_status);
                if status & NVME_CSTS_FATAL != 0 {
                    return Err(DriverError::ControllerFault)
                }
                if status & NVME_CSTS_READY != 0 {
                    return Ok(())
                }
                if spins == 0 {
                    return Err(DriverError::TimedOut)
                }
                spins -= 1;
                core::hint::spin_loop()
            }
        }
    }

    /// Returns the first submission-queue doorbell.
    pub fn admin_submission_doorbell(&mut self) -> *mut u32 {
        unsafe { self.registers.as_ptr().cast::<u8>().add(0x1000).cast() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvmeCommand {
    pub opcode_and_flags: u32,
    pub namespace_id: u32,
    pub reserved: [u32; 2],
    pub metadata: u64,
    pub data_pointer_1: u64,
    pub data_pointer_2: u64,
    pub command_data: [u32; 6],
}

impl NvmeCommand {
    pub const fn identify(controller_data_physical: u64) -> Self {
        Self {
            opcode_and_flags: 0x06,
            namespace_id: 0,
            reserved: [0; 2],
            metadata: 0,
            data_pointer_1: controller_data_physical,
            data_pointer_2: 0,
            command_data: [1, 0, 0, 0, 0, 0],
        }
    }

    pub const fn read(
        namespace_id: u32,
        lba: u64,
        zero_based_block_count: u16,
        data_physical: u64,
    ) -> Self {
        Self::io(
            0x02,
            namespace_id,
            lba,
            zero_based_block_count,
            data_physical,
        )
    }

    pub const fn write(
        namespace_id: u32,
        lba: u64,
        zero_based_block_count: u16,
        data_physical: u64,
    ) -> Self {
        Self::io(
            0x01,
            namespace_id,
            lba,
            zero_based_block_count,
            data_physical,
        )
    }

    const fn io(
        opcode: u32,
        namespace_id: u32,
        lba: u64,
        block_count: u16,
        data_physical: u64,
    ) -> Self {
        Self {
            opcode_and_flags: opcode,
            namespace_id,
            reserved: [0; 2],
            metadata: 0,
            data_pointer_1: data_physical,
            data_pointer_2: 0,
            command_data: [
                lba as u32,
                (lba >> 32) as u32,
                block_count as u32,
                0,
                0,
                0,
            ],
        }
    }

    pub fn set_command_id(&mut self, command_id: u16) {
        self.opcode_and_flags =
            self.opcode_and_flags & 0x0000_ffff | (command_id as u32) << 16
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvmeCompletion {
    pub result: u32,
    pub reserved: u32,
    pub submission_head: u16,
    pub submission_queue_id: u16,
    pub command_id: u16,
    pub status: u16,
}

pub struct NvmeQueue {
    submission: NonNull<NvmeCommand>,
    completion: NonNull<NvmeCompletion>,
    submission_doorbell: NonNull<u32>,
    completion_doorbell: NonNull<u32>,
    depth: u16,
    submission_tail: u16,
    completion_head: u16,
    phase: bool,
    next_command_id: u16,
}

impl NvmeQueue {
    /// # Safety
    /// Queue pointers must address initialized, controller-owned DMA arrays of
    /// `depth` entries. Doorbells must belong to the same queue.
    pub unsafe fn new(
        submission: *mut NvmeCommand,
        completion: *mut NvmeCompletion,
        submission_doorbell: *mut u32,
        completion_doorbell: *mut u32,
        depth: u16,
    ) -> Result<Self, DriverError> {
        if depth < 2 {
            return Err(DriverError::InvalidQueue)
        }
        Ok(Self {
            submission: NonNull::new(submission).ok_or(DriverError::InvalidQueue)?,
            completion: NonNull::new(completion).ok_or(DriverError::InvalidQueue)?,
            submission_doorbell: NonNull::new(submission_doorbell)
                .ok_or(DriverError::InvalidQueue)?,
            completion_doorbell: NonNull::new(completion_doorbell)
                .ok_or(DriverError::InvalidQueue)?,
            depth,
            submission_tail: 0,
            completion_head: 0,
            phase: true,
            next_command_id: 0,
        })
    }

    pub fn submit(
        &mut self,
        mut command: NvmeCommand,
        spin_limit: usize,
    ) -> Result<NvmeCompletion, DriverError> {
        let command_id = self.next_command_id;
        self.next_command_id = self.next_command_id.wrapping_add(1);
        command.set_command_id(command_id);

        unsafe {
            write_volatile(
                self.submission.as_ptr().add(self.submission_tail as usize),
                command,
            );
            self.submission_tail = (self.submission_tail + 1) % self.depth;
            write_volatile(
                self.submission_doorbell.as_ptr(),
                self.submission_tail as u32,
            )
        }

        let mut spins = spin_limit;
        loop {
            let completion = unsafe {
                read_volatile(self.completion.as_ptr().add(self.completion_head as usize))
            };
            if (completion.status & 1 != 0) == self.phase {
                if completion.command_id != command_id {
                    return Err(DriverError::ControllerFault)
                }

                self.completion_head += 1;
                if self.completion_head == self.depth {
                    self.completion_head = 0;
                    self.phase = !self.phase
                }
                unsafe {
                    write_volatile(
                        self.completion_doorbell.as_ptr(),
                        self.completion_head as u32,
                    )
                }

                if completion.status >> 1 & 0x7ff != 0 {
                    return Err(DriverError::ControllerFault)
                }
                return Ok(completion)
            }

            if spins == 0 {
                return Err(DriverError::TimedOut)
            }
            spins -= 1;
            core::hint::spin_loop()
        }
    }
}
