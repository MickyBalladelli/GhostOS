pub mod cpu;
pub mod memory;
pub mod devices;
pub mod firmware;
pub mod boot;

pub use cpu::{Cpu, CpuState, CpuMode, PrivilegeLevel, CpuError};
pub use memory::Mmu;
pub use devices::{InterruptController, Device};
pub use firmware::bios::{Bios, BiosContext};
pub use boot::Loader;

use std::path::PathBuf;

pub struct VmConfig {
    pub memory_size: usize,
    pub kernel_path: Option<PathBuf>,
    pub initrd_path: Option<PathBuf>,
    pub boot_args: String,
    pub smp_cores: usize,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            memory_size: 128 * 1024 * 1024,
            kernel_path: None,
            initrd_path: None,
            boot_args: String::new(),
            smp_cores: 1,
        }
    }
}

pub struct Vm {
    cpu: Cpu,
    mmu: Mmu,
    interrupt_controller: InterruptController,
    bios: Bios,
    config: VmConfig,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            mmu: Mmu::new(128 * 1024 * 1024),
            interrupt_controller: InterruptController::new(),
            bios: Bios::new(),
            config: VmConfig::default(),
        }
    }

    pub fn with_config(config: VmConfig) -> Self {
        Self {
            cpu: Cpu::new(),
            mmu: Mmu::new(config.memory_size),
            interrupt_controller: InterruptController::new(),
            bios: Bios::new(),
            config,
        }
    }

    pub fn run(&mut self) -> Result<(), VmError> {
        println!("Initializing VM...");
        
        let _ = self.bios.init(&mut self.mmu, &mut self.cpu);
        
        println!("Starting CPU emulation...");
        
        loop {
            self.cpu.step(&mut self.mmu, &mut self.interrupt_controller, &mut self.bios.context)?;
            
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
        self.bios.reset();
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