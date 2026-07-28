#![no_std]

//! Heap-free driver building blocks for legacy x86_64 PCs.
//!
//! The crate contains no kernel policy. A Ring 3 driver service supplies its
//! capability-authorized port and MMIO mappings, then uses these structures to
//! discover and operate the device.

pub mod ethernet;
pub mod pci;
pub mod storage;

pub use ethernet::{EthernetAdapter, EthernetKind};
pub use pci::{Bar, ConfigAccess, PciAddress, PciDevice};
pub use storage::{StorageController, StorageKind};

