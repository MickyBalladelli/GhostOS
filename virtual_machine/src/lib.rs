pub mod cpu;
pub mod memory;
pub mod devices;
pub mod firmware;
pub mod boot;

pub use cpu::{Cpu, CpuState, CpuMode, PrivilegeLevel, CpuError};
pub use memory::Mmu;
pub use devices::{InterruptController, Device, PortBus, Serial16550, PciHostBridge, PciDeviceId};
pub use firmware::bios::{Bios, BiosContext};
pub use boot::Loader;

use std::path::PathBuf;

pub const COM1_PORT: u16 = 0x3F8;

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
    bios: Bios,
    config: VmConfig,
}

impl Vm {
    pub fn new() -> Self {
        Self::with_config(VmConfig::default())
    }

    pub fn with_config(config: VmConfig) -> Self {
        let mut ports = PortBus::new();
        if config.enable_serial {
            ports.attach(COM1_PORT, 8, Box::new(Serial16550::new(COM1_PORT)));
        }
        Self {
            cpu: Cpu::new(),
            mmu: Mmu::new(config.memory_size),
            interrupt_controller: InterruptController::new(),
            ports,
            bios: Bios::new(),
            config,
        }
    }

    pub fn run(&mut self) -> Result<(), VmError> {
        println!("Initializing VM...");

        let _ = self.bios.init(&mut self.mmu, &mut self.cpu.state);

        println!("Starting CPU emulation...");

        loop {
            self.cpu.step(
                &mut self.mmu,
                &mut self.interrupt_controller,
                &mut self.ports,
                &mut self.bios.context,
            )?;

            if self.cpu.state.halted {
                println!("CPU halted");
                break;
            }
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.mmu.reset();
        self.interrupt_controller.reset();
        self.ports.reset();
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