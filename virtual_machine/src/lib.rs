pub mod cpu;
pub mod memory;
pub mod devices;
pub mod firmware;
pub mod boot;
pub mod net;

pub use cpu::{Cpu, CpuState, CpuMode, PrivilegeLevel, CpuError};
pub use memory::Mmu;
pub use net::{LoopbackHub, LoopbackPort, MacAddress, NetBackend, PacketQueue};
pub use devices::{
    Ahci, ApicTrigger, Device, DiskImage, DisplayState, E1000, E1000_MMIO_SIZE, GopMode,
    GopPixelFormat, Hpet, InterruptController, LocalApic, Nvme, PciDeviceId, PciHostBridge, Pit,
    PortBus, PortDevice, Serial16550, UefiGop, VesaFbDevice, VgaPorts, VgaTextDevice, VideoMode,
    VirtioNet,
    AHCI_ABAR_SIZE, AHCI_CLASS, AHCI_DEVICE_ID, AHCI_PROG_IF, AHCI_SUBCLASS, AHCI_VENDOR_ID,
    APIC_BASE_DEFAULT, APIC_SIZE, HPET_BASE_DEFAULT, HPET_SIZE, IA32_APIC_BASE_MSR,
    NVME_BAR0_SIZE, NVME_CLASS, NVME_DEVICE_ID, NVME_PROG_IF, NVME_SUBCLASS, NVME_VENDOR_ID,
    PCIE_ECAM_BASE_DEFAULT, PIT_CH0_PORT, PIT_PORT_COUNT, VBE_MODES, VESA_FB_SIZE, VESA_LFB_BASE,
    VGA_PORT_BASE, VGA_PORT_COUNT, VGA_TEXT_BASE, VGA_TEXT_SIZE,
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

/// AHCI controller ABAR (BAR5) aperture base.
pub const AHCI_MMIO_BASE: u64 = 0xF100_0000;
/// NVMe controller BAR0 aperture base.
pub const NVME_MMIO_BASE: u64 = 0xF110_0000;
pub const E1000_MMIO_BASE: u64 = 0xF120_0000;
pub const VIRTIO_NET_IO_BASE: u16 = 0x5000;

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
    pit: Rc<RefCell<Pit>>,
    hpet: Rc<RefCell<Hpet>>,
    ahci: Rc<RefCell<Ahci>>,
    nvme: Rc<RefCell<Nvme>>,
    e1000: Rc<RefCell<E1000>>,
    virtio_net: Rc<RefCell<VirtioNet>>,
    display: Rc<RefCell<DisplayState>>,
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

        // 8254 PIT on the legacy I/O ports 0x40..0x43. Channel 0 maps to
        // ISA IRQ0 which the I/O APIC redirects to APIC vector 0x20 in the
        // default PC-compatible interrupt table.
        let pit: Rc<RefCell<Pit>> = Rc::new(RefCell::new(Pit::new()));
        pit.borrow_mut().attach_apic(apic.clone());
        pit.borrow_mut().set_irq0_vector(0x20);
        ports.attach(PIT_CH0_PORT, PIT_PORT_COUNT, Box::new(pit.clone()));

        // HPET at the ACPI base 0xFED0_0000. Legacy-replacement interrupts
        // from timers 0/1 are routed to APIC vector 0x20 (IRQ0) and 0x21
        // (IRQ1) matching the IA-PC compatibility table.
        let hpet: Rc<RefCell<Hpet>> = Rc::new(RefCell::new(Hpet::new()));
        hpet.borrow_mut().attach_apic(apic.clone());
        hpet.borrow_mut().set_legacy_vector(0x20);
        mmu.attach_mmio(HPET_BASE_DEFAULT, HPET_SIZE, Box::new(hpet.clone()));

        // -------------------------------------------------------------------
        // Storage controllers
        // -------------------------------------------------------------------

        // AHCI SATA HBA at device 4, function 0. BAR5 (ABAR) is memory
        // mapped at AHCI_MMIO_BASE (4 KiB) and interrupts to vector 0x2B
        // (ISA IRQ11 remapped by the I/O APIC).
        let ahci: Rc<RefCell<Ahci>> = Rc::new(RefCell::new(Ahci::new()));
        ahci.borrow_mut().attach_apic(apic.clone());
        ahci.borrow_mut().set_irq_vector(0x2B);
        mmu.attach_mmio(AHCI_MMIO_BASE, AHCI_ABAR_SIZE, Box::new(ahci.clone()));
        pci.borrow_mut().add_device(
            0,
            4,
            0,
            PciDeviceId {
                vendor: AHCI_VENDOR_ID,
                device: AHCI_DEVICE_ID,
                revision: 0x01,
                prog_if: AHCI_PROG_IF,
                subclass: AHCI_SUBCLASS,
                class: AHCI_CLASS,
            },
        );
        pci.borrow_mut().set_bar_size(0, 4, 0, 5, AHCI_ABAR_SIZE as u32).ok();
        pci.borrow_mut().write_config(0, 4, 0, 0x10 + 5 * 4, AHCI_MMIO_BASE as u32);

        // NVMe controller at device 5, function 0. BAR0 is memory mapped at
        // NVME_MMIO_BASE (8 KiB) and interrupts to vector 0x31 (IRQ17).
        let nvme: Rc<RefCell<Nvme>> = Rc::new(RefCell::new(Nvme::new()));
        nvme.borrow_mut().attach_apic(apic.clone());
        nvme.borrow_mut().set_irq_vector(0x31);
        mmu.attach_mmio(NVME_MMIO_BASE, NVME_BAR0_SIZE, Box::new(nvme.clone()));
        pci.borrow_mut().add_device(
            0,
            5,
            0,
            PciDeviceId {
                vendor: NVME_VENDOR_ID,
                device: NVME_DEVICE_ID,
                revision: 0x01,
                prog_if: NVME_PROG_IF,
                subclass: NVME_SUBCLASS,
                class: NVME_CLASS,
            },
        );
        pci.borrow_mut().set_bar_size(0, 5, 0, 0, NVME_BAR0_SIZE as u32).ok();
        pci.borrow_mut().write_config(0, 5, 0, 0x10, NVME_MMIO_BASE as u32);

        // Networking: e1000 + virtio-net on a loopback hub.
        let hub = Rc::new(RefCell::new(LoopbackHub::new()));
        let e1000_mac = MacAddress::synos_default(0x56);
        let virtio_mac = MacAddress::synos_default(0x57);
        let e1000 =
            Rc::new(RefCell::new(E1000::new(e1000_mac)));
        e1000.borrow_mut().attach_apic(apic.clone());
        e1000.borrow_mut().set_irq_vector(0x2D);
        e1000.borrow_mut().attach_backend(Box::new(
            LoopbackPort::new(hub.clone(), 0, e1000_mac),
        ));
        mmu.attach_mmio(E1000_MMIO_BASE, E1000_MMIO_SIZE, Box::new(e1000.clone()));
        pci.borrow_mut().add_device(
            0, 6, 0,
            PciDeviceId {
                vendor: 0x8086, device: 0x100E, revision: 0x03,
                prog_if: 0x00, subclass: 0x00, class: 0x02,
            },
        );
        pci.borrow_mut()
            .set_bar_size(0, 6, 0, 0, E1000_MMIO_SIZE as u32)
            .ok();
        pci.borrow_mut()
            .write_config(0, 6, 0, 0x10, E1000_MMIO_BASE as u32);

        let virtio_net = Rc::new(RefCell::new(VirtioNet::new(virtio_mac)));
        virtio_net.borrow_mut().attach_apic(apic.clone());
        virtio_net.borrow_mut().set_irq_vector(0x2E);
        virtio_net.borrow_mut().attach_backend(Box::new(
            LoopbackPort::new(hub.clone(), 1, virtio_mac),
        ));
        ports.attach(VIRTIO_NET_IO_BASE, 0x20, Box::new(virtio_net.clone()));
        pci.borrow_mut().add_device(
            0, 7, 0,
            PciDeviceId {
                vendor: 0x1AF4, device: 0x1000, revision: 0x01,
                prog_if: 0x00, subclass: 0x00, class: 0x02,
            },
        );
        pci.borrow_mut().set_bar_size(0, 7, 0, 0, 0x100).ok();
        pci.borrow_mut()
            .write_config(0, 7, 0, 0x10, VIRTIO_NET_IO_BASE as u32 | 0x1);

        let mut cpu = Cpu::new();
        cpu.attach_apic(apic.clone());

        // Display: shared VGA text buffer + VESA LFB + VGA controller ports.
        let display: Rc<RefCell<DisplayState>> = Rc::new(RefCell::new(DisplayState::new()));
        mmu.attach_mmio(
            VGA_TEXT_BASE,
            VGA_TEXT_SIZE as u64,
            Box::new(VgaTextDevice::new(display.clone())),
        );
        mmu.attach_mmio(
            VESA_LFB_BASE,
            VESA_FB_SIZE as u64,
            Box::new(VesaFbDevice::new(display.clone())),
        );
        ports.attach(
            VGA_PORT_BASE,
            VGA_PORT_COUNT,
            Box::new(VgaPorts::new(display.clone())),
        );

        let mut bios = Bios::new();
        bios.context.set_display(display.clone());

        Self {
            cpu,
            mmu,
            interrupt_controller: InterruptController::new(),
            ports,
            pci,
            apic,
            pit,
            hpet,
            ahci,
            nvme,
            e1000,
            virtio_net,
            display,
            bios,
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

            // Deferred DMA for storage and NICs issued during the step.
            self.poll_devices();

            let now_ns = started.elapsed().as_nanos() as u64;
            self.poll_apic(now_ns)?;

            if self.cpu.state.halted {
                println!("CPU halted");
                break;
            }
        }

        Ok(())
    }

    /// Process deferred DMA for storage controllers and NICs issued during
    /// the last CPU step. Runs outside the executor's `&mut Mmu` borrow so
    /// devices can DMA directly into guest physical memory.
    fn poll_devices(&mut self) {
        if self.ahci.borrow().has_pending() {
            self.ahci.borrow_mut().poll_dma(&mut self.mmu);
        }
        if self.nvme.borrow().has_pending() {
            self.nvme.borrow_mut().poll_dma(&mut self.mmu);
        }
        self.e1000.borrow_mut().poll(&mut self.mmu);
        if self.virtio_net.borrow().has_pending() {
            self.virtio_net.borrow_mut().poll(&mut self.mmu);
        }
    }

    /// Advance the local APIC timer plus the PIT/HPET timebase and deliver
    /// the highest-priority pending vector when the CPU can take an interrupt
    /// (IF set, outside the STI shadow window).
    fn poll_apic(&mut self, now_ns: u64) -> Result<(), VmError> {
        // Each timer is advanced against host time; expired counters signal
        // their APIC vector, which lands in the IRR priority queue.
        self.pit.borrow_mut().advance(now_ns);
        self.hpet.borrow_mut().advance(now_ns);
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
        self.pit.borrow_mut().reset();
        self.hpet.borrow_mut().reset();
        self.ahci.borrow_mut().reset();
        self.nvme.borrow_mut().reset();
        self.display.borrow_mut().reset();
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

    /// Shared handle to the 8254 PIT (port-mapped at 0x40..0x43).
    pub fn pit(&self) -> Rc<RefCell<Pit>> {
        self.pit.clone()
    }

    /// Shared handle to the HPET (MMIO at 0xFED0_0000).
    pub fn hpet(&self) -> Rc<RefCell<Hpet>> {
        self.hpet.clone()
    }

    /// Shared handle to the AHCI SATA host controller.
    pub fn ahci(&self) -> Rc<RefCell<Ahci>> {
        self.ahci.clone()
    }

    /// Shared handle to the NVMe controller.
    pub fn nvme(&self) -> Rc<RefCell<Nvme>> {
        self.nvme.clone()
    }

    /// Shared handle to the display state (VGA text, VESA LFB, palette).
    pub fn display(&self) -> Rc<RefCell<DisplayState>> {
        self.display.clone()
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