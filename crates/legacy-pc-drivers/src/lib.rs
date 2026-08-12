#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

//! Heap-free driver building blocks for legacy x86_64 PCs.
//!
//! The crate contains no kernel policy. A Ring 3 driver service supplies its
//! capability-authorized port and MMIO mappings, then uses these structures to
//! discover and operate the device.

pub mod ethernet;
pub mod block;
pub mod pci;
pub mod storage;
pub mod usb;

pub use ethernet::{
    EthernetAdapter, EthernetKind, EthernetQueueSnapshot, EthernetRuntime, EthernetSnapshot,
};
pub use block::{AhciBlockDevice, NvmeBlockDevice};
pub use pci::{
    Bar, ConfigAccess, ExtendedConfigAccess, PCIE_AER_EXTENDED_CAPABILITY, PciAddress, PciDevice,
    PcieAerStatus, clear_pcie_aer, read_pcie_aer,
};
pub use storage::{
    MAX_NVME_NAMESPACES, NvmeNamespace, NvmeNamespaceState, NvmeRegistry, StorageController,
    StorageKind,
};
pub use usb::{
    BotCommandBlockWrapper, BotCommandStatusWrapper, HidKeyboardReport, HidMouseReport,
    ScsiCommand,
};
