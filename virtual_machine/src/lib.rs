pub mod cpu;
pub mod memory;
pub mod devices;
pub mod firmware;
pub mod boot;

pub use cpu::{Cpu, CpuState, CpuMode, PrivilegeLevel, CpuError};
pub use memory::Mmu;
pub use devices::{
    ApicTrigger, InterruptController, LocalApic, Device, PortBus, Serial16550, PciHostBridge,
    PciDeviceId, APIC_BASE_DEFAULT, APIC_SIZE, IA32_APIC_BASE_MSR, PCIE_ECAM_BASE_DEFAULT,
};
pub use firmware::bios::{Bios, BiosContext};
pub use boot::Loader;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub const COM1_PORT: u16 = 0x3F8;
pub const PCI_CONFIG_PORT: u16 = 0xCF8;
pub const PCI_CONFIG_PORT_SIZE: u16 = 8;

/// Size of the ECAM (MMCONFIG) aperture: 256 buses * 32 devices * 8 funcs * 4 KiB.
pub const PCIE_ECAM_SIZE: u64 = 256 * 32 * 8 * 4096;

pub struct VmConfig {
    pub memory_size: usize,
    pub kernel_path: Option<PathBuf>,
    pub initrd_path: Option<PathBuf>,
    pub boot_args: String,
    pub smp_cores: usize,
    pub enable_serial: bool,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            memory_size: 128 * 1024 * 1024,
            kernel_path: None,
            initrd_path: None,
            boot_args: String::new(),
            smp_cores: 1,
            enable_serial: true,
        }
    }
}

pub struct Vm {
    cpu: Cpu,
    mmu: Mmu,
    interrupt_controller: InterruptController,
    ports: PortBus,
    pci: Rc<RefCell<PciHostBridge>>,
    apic: Rc<RefCell<LocalApic>>,
    bios: Bios,
    config: VmConfig,
}

impl Vm {
    pub fn new() -> Self {
        Self::with_config(VmConfig::default())
    }

    pub fn with_config(config: VmConfig) -> Self {
        let mut mmu = Mmu::new(config.memory_size);

        // One shared PCI Express host bridge exposed through both the legacy
        // 0xCF8/0xCFC config ports and the ECAM (MMCONFIG) memory aperture.
        let pci: Rc<RefCell<PciHostBridge>> = Rc::new(RefCell::new(PciHostBridge::new()));

        let mut ports = PortBus::new();
        if config.enable_serial {
            ports.attach(COM1_PORT, 8, Box::new(Serial16550::new(COM1_PORT)));
        }
        ports.attach(PCI_CONFIG_PORT, PCI_CONFIG_PORT_SIZE, Box::new(pci.clone()));

        // ECAM aperture is memory-mapped; route it through the MMU so guest
        // loads/stores to the configuration space hit the same bridge.
        mmu.attach_mmio(PCIE_ECAM_BASE_DEFAULT, PCIE_ECAM_SIZE, Box::new(pci.clone()));

        // One local APIC (BSP id 0) shared between the CPU (for the
        // IA32_APIC_BASE MSR path) and the MMU (for the xAPIC MMIO path).
        let apic: Rc<RefCell<LocalApic>> = Rc::new(RefCell::new(LocalApic::new(0)));
        mmu.attach_mmio(APIC_BASE_DEFAULT, APIC_SIZE, Box::new(apic.clone()));

        let mut cpu = Cpu::new();
        cpu.attach_apic(apic.clone());

        Self {
            cpu,
            mmu,
            interrupt_controller: InterruptController::new(),
            ports,
            pci,
            apic,
            bios: Bios::new(),
            config,
        }
    }

    pub fn run(&mut self) -> Result<(), VmError> {
        println!("Initializing VM...");

        let _ = self.bios.init(&mut self.mmu, &mut self.cpu.state);

        println!("Starting CPU emulation...");

        let started = std::time::Instant::now();
        loop {
            self.cpu.step(
                &mut self.mmu,
                &mut self.interrupt_controller,
                &mut self.ports,
                &mut self.bios.context,
            )?;

            let now_ns = started.elapsed().as_nanos() as u64;
            self.poll_apic(now_ns)?;

            if self.cpu.state.halted {
                println!("CPU halted");
                break;
            }
        }

        Ok(())
    }

    /// Advance the local APIC timer and deliver the highest-priority pending
    /// vector when the CPU can take an interrupt (IF set, outside the STI
    /// shadow window).
    fn poll_apic(&mut self, now_ns: u64) -> Result<(), VmError> {
        // Timer is advanced against host time; the count-down wraps into the
        // IRR priority queue on expiry.
        self.apic.borrow_mut().advance(now_ns);

        let pending = self.apic.borrow_mut().pending_vector();
        let Some(vector) = pending else {
            return Ok(());
        };

        // Maskable interrupts are only taken when the guest has sti'd and is
        // outside the one-instruction STI shadow window.
        if self.cpu.state.rflags & (1 << 9) == 0 || self.cpu.state.interrupt_shadow {
            return Ok(());
        }

        self.apic.borrow_mut().accept_pending(vector);
        self.cpu
            .handle_interrupt(
                vector,
                &mut self.mmu,
                &mut self.interrupt_controller,
            )?;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.mmu.reset();
        self.interrupt_controller.reset();
        self.ports.reset();
        self.pci.borrow_mut().reset();
        self.apic.borrow_mut().reset();
        self.bios.reset();
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    pub fn mmu(&self) -> &Mmu {
        &self.mmu
    }

    pub fn mmu_mut(&mut self) -> &mut Mmu {
        &mut self.mmu
    }

    /// Shared handle to the PCI Express host bridge (both the port-mapped
    /// config path and the ECAM/MMCONFIG path use the same bridge).
    pub fn pci(&self) -> Rc<RefCell<PciHostBridge>> {
        self.pci.clone()
    }

    /// Shared handle to the local APIC (MMIO and IA32_APIC_BASE MSR paths).
    pub fn apic(&self) -> Rc<RefCell<LocalApic>> {
        self.apic.clone()
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum VmError {
    CpuError(CpuError),
    MemoryError,
    BiosError,
    IoError,
    InvalidConfiguration,
    KernelLoadError,
    BootFailure,
}

impl From<CpuError> for VmError {
    fn from(e: CpuError) -> Self {
        VmError::CpuError(e)
    }
}