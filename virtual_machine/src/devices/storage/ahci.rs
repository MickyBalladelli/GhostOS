//! AHCI (Advanced Host Controller Interface) SATA HBA emulation.
//!
//! Single-port SATA HBA exposing the standard register file (ABAR 4 KiB),
//! command list + PRD DMA, H2D/D2H register FIS protocol, and IDENTIFY +
//! DMA read/write ATA commands backed by a [`DiskImage`].
//!
//! Commands are recorded when the guest issues them (CI write); the actual
//! DMA transfer is deferred to [`Ahci::poll_dma`], called by the machine
//! outside the CPU step so it can borrow guest physical memory safely.

use crate::devices::storage::{DiskImage, StorageError};
use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::rc::Rc;

pub const AHCI_ABAR_SIZE: u64 = 0x1000;
pub const AHCI_VENDOR_ID: u16 = 0x8086;
pub const AHCI_DEVICE_ID: u16 = 0x2922;
pub const AHCI_CLASS: u8 = 0x01;
pub const AHCI_SUBCLASS: u8 = 0x06;
pub const AHCI_PROG_IF: u8 = 0x01;

const HBA_CAP: u32 = 0x00;
const HBA_GHC: u32 = 0x04;
const HBA_IS: u32 = 0x08;
const HBA_PI: u32 = 0x0C;
const HBA_VS: u32 = 0x10;
const HBA_CAP2: u32 = 0x24;
const HBA_BOHC: u32 = 0x28;

const PORT_CLB: u32 = 0x00;
const PORT_CLBU: u32 = 0x04;
const PORT_FB: u32 = 0x08;
const PORT_FBU: u32 = 0x0C;
const PORT_IS: u32 = 0x10;
const PORT_IE: u32 = 0x14;
const PORT_CMD: u32 = 0x18;
const PORT_TFD: u32 = 0x20;
const PORT_SIG: u32 = 0x24;
const PORT_STS: u32 = 0x28;
const PORT_SCTL: u32 = 0x2C;
const PORT_SERR: u32 = 0x30;
const PORT_SACT: u32 = 0x34;
const PORT_CI: u32 = 0x38;
const PORT_SNTF: u32 = 0x3C;

const IS_DHRS: u32 = 1 << 0;
const IS_DPS: u32 = 1 << 5;

const GHC_AE: u32 = 1 << 31;
const GHC_IE: u32 = 1 << 1;
const GHC_HR: u32 = 1 << 0;
const CMD_CR: u32 = 1 << 15;

const FIS_H2D: u8 = 0x27;
const FIS_D2H: u8 = 0x34;
const FIS_PIO: u8 = 0x5F;

const CMD_HEADER_SIZE: u64 = 32;
const MAX_TRANSFER_BYTES: usize = 16 * 1024 * 1024;

const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ: u8 = 0x20;
const ATA_READ_EXT: u8 = 0x24;
const ATA_READ_DMA: u8 = 0xC8;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE: u8 = 0x30;
const ATA_WRITE_EXT: u8 = 0x34;
const ATA_WRITE_DMA: u8 = 0xCA;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH: u8 = 0xE7;
const ATA_FLUSH_EXT: u8 = 0xEA;
const ATA_SET_FEATURES: u8 = 0xEF;
const ATA_SET_MULTIPLE: u8 = 0xC6;

struct AhciPort {
    clb: u64,
    fb: u64,
    is: u32,
    ie: u32,
    cmd: u32,
    tfd: u32,
    sig: u32,
    sts: u32,
    sctl: u32,
    serr: u32,
    sact: u32,
    ci: u32,
    sntf: u32,
    pending: u32,
    disk: Option<DiskImage>,
}

impl AhciPort {
    fn new() -> Self {
        Self {
            clb: 0,
            fb: 0,
            is: 0,
            ie: 0,
            cmd: 0,
            tfd: 0,
            sig: 0x0000_0101,
            sts: 0x0123,
            sctl: 0,
            serr: 0,
            sact: 0,
            ci: 0,
            sntf: 0,
            pending: 0,
            disk: None,
        }
    }
}

pub struct Ahci {
    cap: u32,
    ghc: u32,
    is: u32,
    pi: u32,
    vs: u32,
    cap2: u32,
    bohc: u32,
    port: AhciPort,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Ahci {
    pub fn new() -> Self {
        Self {
            cap: (1 << 0) | (3 << 8) | (1 << 17) | (1 << 31),
            ghc: 0,
            is: 0,
            pi: 1,
            vs: 0x0001_0200,
            cap2: 0,
            bohc: 0,
            port: AhciPort::new(),
            apic: None,
            irq_vector: 0,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector;
    }

    pub fn attach_disk(&mut self, disk: DiskImage) -> Option<DiskImage> {
        self.port.disk.replace(disk)
    }

    pub fn detach_disk(&mut self) -> Option<DiskImage> {
        self.port.disk.take()
    }

    pub fn flush_disk(&mut self) -> Result<(), StorageError> {
        self.port
            .disk
            .as_mut()
            .ok_or_else(|| StorageError::InvalidImage("no disk".into()))?
            .flush()
    }

    pub fn sync_disk(&mut self) -> Result<(), StorageError> {
        self.port
            .disk
            .as_mut()
            .ok_or_else(|| StorageError::InvalidImage("no disk".into()))?
            .sync()
    }

    pub fn sector_count(&self) -> Option<u64> {
        self.port.disk.as_ref().map(|d| d.sector_count())
    }

    pub fn has_pending(&self) -> bool {
        self.port.pending != 0
    }

    /// Process all previously-issued commands. `mmu` provides physical-memory
    /// DMA access; call this after each CPU step.
    pub fn poll_dma(&mut self, mmu: &mut Mmu) {
        let issued = std::mem::take(&mut self.port.pending);
        if issued == 0 {
            return;
        }
        let mut raise = 0u32;
        for slot in 0..32 {
            let bit = 1u32 << slot;
            if issued & bit == 0 {
                continue;
            }
            match self.process_slot(slot, mmu) {
                Ok(extra) => raise |= extra | IS_DHRS,
                Err(_) => {
                    self.port.tfd = 0x01;
                    self.port.serr |= 1;
                    raise |= IS_DHRS;
                }
            }
        }
        self.port.ci &= !issued;
        self.is |= raise;
        self.port.is |= raise;
        self.raise_irq();
    }

    fn dma_read(mmu: &Mmu, addr: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        let bytes = mmu
            .read_phys(addr, buf.len())
            .map_err(|_| StorageError::Dma("read".into()))?;
        buf.copy_from_slice(&bytes);
        Ok(())
    }

    fn dma_write(mmu: &mut Mmu, addr: u64, buf: &[u8]) -> Result<(), StorageError> {
        mmu.write_phys(addr, buf)
            .map_err(|_| StorageError::Dma("write".into()))
    }

    fn read_host(&self, off: u32) -> u32 {
        match off {
            HBA_CAP => self.cap,
            HBA_GHC => self.ghc,
            HBA_IS => self.is,
            HBA_PI => self.pi,
            HBA_VS => self.vs,
            HBA_CAP2 => self.cap2,
            HBA_BOHC => self.bohc,
            _ => 0,
        }
    }

    fn write_host(&mut self, off: u32, value: u32) {
        match off {
            HBA_GHC if value & GHC_HR != 0 => {
                let apic = self.apic.take();
                let v = self.irq_vector;
                let disk = self.port.disk.take();
                *self = Self::new();
                self.apic = apic;
                self.irq_vector = v;
                self.port.disk = disk;
            }
            HBA_GHC => {
                self.ghc = (self.ghc & !(GHC_AE | GHC_IE)) | (value & (GHC_AE | GHC_IE));
            }
            HBA_IS => {
                self.is &= !value;
                self.port.is &= !value;
            }
            _ => {}
        }
    }

    fn read_port(&self, off: u32) -> u32 {
        let p = &self.port;
        match off {
            PORT_CLB => p.clb as u32,
            PORT_CLBU => (p.clb >> 32) as u32,
            PORT_FB => p.fb as u32,
            PORT_FBU => (p.fb >> 32) as u32,
            PORT_IS => p.is,
            PORT_IE => p.ie,
            PORT_CMD => p.cmd,
            PORT_TFD => p.tfd,
            PORT_SIG => p.sig,
            PORT_STS => p.sts,
            PORT_SCTL => p.sctl,
            PORT_SERR => p.serr,
            PORT_SACT => p.sact,
            PORT_CI => p.ci,
            PORT_SNTF => p.sntf,
            _ => 0,
        }
    }

    fn write_port(&mut self, off: u32, value: u32) {
        let p = &mut self.port;
        match off {
            PORT_CLB => p.clb = (p.clb & !0xFFFF_FFFF) | value as u64,
            PORT_CLBU => p.clb = (p.clb & 0xFFFF_FFFF) | (value as u64) << 32,
            PORT_FB => p.fb = (p.fb & !0xFFFF_FFFF) | value as u64,
            PORT_FBU => p.fb = (p.fb & 0xFFFF_FFFF) | (value as u64) << 32,
            PORT_IS => {
                p.is &= !value;
                self.is &= !value;
            }
            PORT_IE => p.ie = value,
            PORT_CMD => p.cmd = (p.cmd & CMD_CR) | (value & !CMD_CR),
            PORT_SCTL => p.sctl = value & 0x0FFF,
            PORT_SERR => p.serr &= !value,
            PORT_SACT => p.sact = value,
            PORT_CI => {
                p.pending |= value & !p.ci;
                p.ci |= value;
            }
            _ => {}
        }
    }

    fn process_slot(&mut self, slot: usize, mmu: &mut Mmu) -> Result<u32, StorageError> {
        if self.port.clb % 1024 != 0 || self.port.fb % 256 != 0 {
            return Err(StorageError::Dma("unaligned AHCI command structures".into()));
        }
        let header_addr = self
            .port
            .clb
            .checked_add(slot as u64 * CMD_HEADER_SIZE)
            .ok_or_else(|| StorageError::Dma("command header address overflow".into()))?;
        let mut header = [0u8; 32];
        Self::dma_read(mmu, header_addr, &mut header)?;

        let Ok(dw0_bytes) = header[0..4].try_into() else {
            return Err(StorageError::Dma("invalid AHCI command header".into()))
        };
        let dw0 = u32::from_le_bytes(dw0_bytes);
        let prdtl = (dw0 & 0xFFFF) as usize;
        if prdtl == 0 || prdtl > 256 {
            return Err(StorageError::Dma("invalid PRDT length".into()));
        }
        if dw0 & (1 << 28) != 0 {
            return Err(StorageError::Unsupported("ATAPI".into()));
        }
        let Ok(ctba_bytes) = header[8..16].try_into() else {
            return Err(StorageError::Dma("invalid AHCI command table address".into()))
        };
        let ctba = u64::from_le_bytes(ctba_bytes);
        if ctba % 128 != 0 {
            return Err(StorageError::Dma("unaligned AHCI command table".into()));
        }

        let mut cfis = [0u8; 64];
        Self::dma_read(mmu, ctba, &mut cfis)?;
        if cfis[0] != FIS_H2D {
            return Err(StorageError::Unsupported("non-H2D FIS".into()));
        }
        let command = cfis[2];
        let sector_count = cfis[12] as u64 | ((cfis[13] as u64) << 8);
        let lba = (cfis[4] as u64 | (cfis[5] as u64) << 8 | (cfis[6] as u64) << 16)
            | (((cfis[8] as u64 | (cfis[9] as u64) << 8 | (cfis[10] as u64) << 16
                | (cfis[11] as u64) << 24))
                << 24);

        let mut prds = Vec::with_capacity(prdtl);
        for i in 0..prdtl {
            let mut prd = [0u8; 16];
            let prd_addr = ctba
                .checked_add(0x80)
                .and_then(|address| address.checked_add((i as u64) * 16))
                .ok_or_else(|| StorageError::Dma("PRD address overflow".into()))?;
            Self::dma_read(mmu, prd_addr, &mut prd)?;
            let Ok(dba_bytes) = prd[0..8].try_into() else {
                return Err(StorageError::Dma("invalid AHCI PRD address".into()))
            };
            let Ok(dbc_bytes) = prd[12..16].try_into() else {
                return Err(StorageError::Dma("invalid AHCI PRD length".into()))
            };
            let dba = u64::from_le_bytes(dba_bytes);
            let dbc = u32::from_le_bytes(dbc_bytes);
            prds.push((dba, (dbc & 0x003F_FFFF) as usize + 1));
        }

        match command {
            ATA_IDENTIFY => {
                let identify = self.build_identify();
                let mut done = 0usize;
                for &(dba, size) in &prds {
                    let n = size.min(identify.len().saturating_sub(done));
                    Self::dma_write(mmu, dba, &identify[done..done + n])?;
                    done += n;
                    if done >= 512 {
                        break;
                    }
                }
                if done != identify.len() {
                    return Err(StorageError::Dma("short IDENTIFY buffer".into()));
                }
                self.write_fis(mmu, FIS_D2H, 0x50, 0, 0x40)?;
                self.write_fis(mmu, FIS_PIO, 0x50, 0, 0x20)?;
                Ok(IS_DPS)
            }
            ATA_READ | ATA_READ_EXT | ATA_READ_DMA | ATA_READ_DMA_EXT => {
                let count = if sector_count == 0 { 256 } else { sector_count as usize };
                if count > MAX_TRANSFER_BYTES / 512 {
                    return Err(StorageError::Dma("transfer too large".into()));
                }
                self.disk_to_prds(mmu, lba, count, &prds, false)?;
                self.write_fis(mmu, FIS_D2H, 0x50, 0, 0x40)?;
                Ok(IS_DPS)
            }
            ATA_WRITE | ATA_WRITE_EXT | ATA_WRITE_DMA | ATA_WRITE_DMA_EXT => {
                let count = if sector_count == 0 { 256 } else { sector_count as usize };
                if count > MAX_TRANSFER_BYTES / 512 {
                    return Err(StorageError::Dma("transfer too large".into()));
                }
                self.disk_to_prds(mmu, lba, count, &prds, true)?;
                self.write_fis(mmu, FIS_D2H, 0x50, 0, 0x40)?;
                Ok(0)
            }
            ATA_FLUSH | ATA_FLUSH_EXT | ATA_SET_FEATURES | ATA_SET_MULTIPLE => {
                if matches!(command, ATA_FLUSH | ATA_FLUSH_EXT) {
                    self.port
                        .disk
                        .as_mut()
                        .ok_or_else(|| StorageError::InvalidImage("no disk".into()))?
                        .flush()?;
                }
                self.write_fis(mmu, FIS_D2H, 0x50, 0, 0x40)?;
                Ok(0)
            }
            other => Err(StorageError::Unsupported(format!("ATA 0x{other:02X}"))),
        }
    }

    /// `to_disk=true`: PRDs -> disk. `false`: disk -> PRDs.
    fn disk_to_prds(
        &mut self,
        mmu: &mut Mmu,
        lba: u64,
        count: usize,
        prds: &[(u64, usize)],
        to_disk: bool,
    ) -> Result<(), StorageError> {
        let disk = self
            .port
            .disk
            .as_mut()
            .ok_or_else(|| StorageError::InvalidImage("no disk".into()))?;
        let mut sector = [0u8; 512];
        let mut idx = 0usize;
        for &(dba, byte_count) in prds {
            let mut remaining = byte_count;
            let mut addr = dba;
            while remaining > 0 && idx < count * 512 {
                let n = remaining.min(512);
                let off = idx % 512;
                if to_disk {
                    Self::dma_read(mmu, addr, &mut sector[off..off + n])?;
                    if off + n == 512 {
                        disk.write_sector(lba + (idx / 512) as u64, &sector)?;
                    }
                } else {
                    if off == 0 {
                        disk.read_sector(lba + (idx / 512) as u64, &mut sector)?;
                    }
                    Self::dma_write(mmu, addr, &sector[off..off + n])?;
                }
                addr = addr
                    .checked_add(n as u64)
                    .ok_or_else(|| StorageError::Dma("PRD address overflow".into()))?;
                remaining -= n;
                idx += n;
            }
            if idx >= count * 512 {
                break;
            }
        }
        let expected = count
            .checked_mul(512)
            .ok_or_else(|| StorageError::Dma("transfer length overflow".into()))?;
        if idx != expected {
            return Err(StorageError::Dma("PRDT length does not match transfer".into()));
        }
        Ok(())
    }

    fn build_identify(&self) -> [u8; 512] {
        let sectors = self.port.disk.as_ref().map(|d| d.sector_count().min(0x0FFF_FFFF)).unwrap_or(0);
        let mut id = [0u8; 512];
        let put = |buf: &mut [u8], w: usize, v: u16| {
            let o = w * 2;
            buf[o..o + 2].copy_from_slice(&v.to_le_bytes());
        };
        put(&mut id, 0, 0x0040);
        put(&mut id, 1, 16383);
        put(&mut id, 3, 16);
        put(&mut id, 6, 63);
        let sn = b"SYNOSVM00001";
        id[20..20 + sn.len()].copy_from_slice(sn);
        let mn = b"SynOS Virtual Disk";
        id[46..46 + mn.len()].copy_from_slice(mn);
        put(&mut id, 47, 0x8001);
        put(&mut id, 49, 0x0F00);
        put(&mut id, 50, 0x4000);
        put(&mut id, 60, (sectors & 0xFFFF) as u16);
        put(&mut id, 61, ((sectors >> 16) & 0xFFFF) as u16);
        put(&mut id, 62, 0x0007);
        put(&mut id, 63, 0x0007);
        put(&mut id, 80, 0x007E);
        put(&mut id, 83, 0x4000);
        put(&mut id, 100, (sectors & 0xFFFF) as u16);
        put(&mut id, 101, ((sectors >> 16) & 0xFFFF) as u16);
        put(&mut id, 102, ((sectors >> 32) & 0xFFFF) as u16);
        put(&mut id, 103, ((sectors >> 48) & 0xFFFF) as u16);
        put(&mut id, 255, 0xA5A5);
        id
    }

    fn write_fis(
        &mut self,
        mmu: &mut Mmu,
        fis_type: u8,
        status: u8,
        error: u8,
        fb_off: u64,
    ) -> Result<(), StorageError> {
        let mut fis = [0u8; 28];
        fis[0] = fis_type;
        fis[1] = 0x02;
        fis[2] = status;
        fis[3] = error;
        fis[12] = 1;
        Self::dma_write(mmu, self.port.fb.checked_add(fb_off).ok_or_else(|| StorageError::Dma("FIS address overflow".into()))?, &fis)
    }

    fn raise_irq(&mut self) {
        if self.ghc & GHC_AE != 0
            && self.ghc & GHC_IE != 0
            && (self.port.is & self.port.ie) != 0
            && self.irq_vector != 0
        {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
            }
        }
    }
}

impl Default for Ahci {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Ahci {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (addr & 0xFFF) as u32;
        let value = if off < 0x100 {
            self.read_host(off)
        } else if (0x100..0x180).contains(&off) {
            self.read_port(off - 0x100)
        } else {
            0
        };
        Ok(value as u64)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (addr & 0xFFF) as u32;
        let value = value as u32;
        if off < 0x100 {
            self.write_host(off, value);
        } else if (0x100..0x180).contains(&off) {
            self.write_port(off - 0x100, value);
        }
        Ok(())
    }

    fn reset(&mut self) {
        let apic = self.apic.take();
        let v = self.irq_vector;
        let disk = self.port.disk.take();
        *self = Self::new();
        self.apic = apic;
        self.irq_vector = v;
        self.port.disk = disk;
    }
}

impl Device for Rc<RefCell<Ahci>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Device::read(&*self.borrow(), addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        Device::write(&mut *self.borrow_mut(), addr, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn image_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("synos-ahci-{name}-{}", std::process::id()))
    }

    fn disk(name: &str) -> DiskImage {
        let path = image_path(name);
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();
        DiskImage::open(path).unwrap()
    }

    #[test]
    fn identification_and_dma_round_trip() {
        let mut ahci = Ahci::new();
        ahci.attach_disk(disk("round-trip"));
        assert_eq!(ahci.sector_count(), Some(8));
        let identify = ahci.build_identify();
        assert_eq!(&identify[46..64], b"SynOS Virtual Disk");

        let mut mmu = Mmu::new(0x20_000);
        let source = 0x1000;
        let target = 0x2000;
        let sector = [0xA5u8; 512];
        mmu.write_phys(source, &sector).unwrap();
        ahci.disk_to_prds(&mut mmu, 2, 1, &[(source, 512)], true).unwrap();
        ahci.disk_to_prds(&mut mmu, 2, 1, &[(target, 512)], false).unwrap();
        assert_eq!(mmu.read_phys(target, 512).unwrap(), sector);
    }

    #[test]
    fn register_access_rejects_bad_size_and_reset_keeps_disk() {
        let mut ahci = Ahci::new();
        ahci.attach_disk(disk("reset"));
        assert_eq!(Device::read(&ahci, 0, 2), Err(DeviceError::UnsupportedSize));
        assert_eq!(Device::read(&ahci, 0, 4).unwrap(), ahci.cap as u64);
        ahci.reset();
        assert_eq!(ahci.sector_count(), Some(8));
        assert!(!ahci.has_pending());
    }
}
