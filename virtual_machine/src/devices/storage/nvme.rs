//! NVMe (Non-Volatile Memory Express) controller emulation.
//!
//! Single-namespace NVMe PCIe device exposing the standard controller
//! registers (BAR0, 8 KiB), the Admin SQ/CQ pair, I/O SQ/CQ pairs, and the
//! core admin commands (IDENTIFY, CREATE/DELETE I/O SQ/CQ, GET/SET
//! FEATURES) plus NVM read/write/flush with PRP DMA.
//!
//! Commands are recorded when the guest rings a submission-queue doorbell;
//! the DMA transfer is deferred to [`Nvme::poll_dma`], called from the
//! machine loop outside the CPU step so guest physical memory can be
//! borrowed without aliasing.

use crate::devices::storage::{DiskImage, StorageError};
use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::rc::Rc;

pub const NVME_BAR0_SIZE: u64 = 0x2000;

/// Intel PCH-style NVMe controller PCI IDs.
pub const NVME_VENDOR_ID: u16 = 0x8086;
pub const NVME_DEVICE_ID: u16 = 0x5845;
pub const NVME_CLASS: u8 = 0x01;
pub const NVME_SUBCLASS: u8 = 0x08;
pub const NVME_PROG_IF: u8 = 0x02;

const MAX_Q_DEPTH: u16 = 1024;

// Controller register offsets (BAR0).
const REG_CAP: u64 = 0x0000;
const REG_VS: u64 = 0x0008;
const REG_INTMS: u64 = 0x000C;
const REG_INTMC: u64 = 0x0010;
const REG_CC: u64 = 0x0014;
const REG_CSTS: u64 = 0x001C;
const REG_AQA: u64 = 0x0024;
const REG_ASQ: u64 = 0x0028;
const REG_ACQ: u64 = 0x0030;
const REG_DBS: u64 = 0x1000;

const CAP_CSS_NVM: u64 = 1 << 37;
const CC_EN: u32 = 1 << 0;
const CSTS_RDY: u32 = 1 << 0;

// Admin command opcodes.
const ADMIN_DELETE_IOSQ: u8 = 0x00;
const ADMIN_CREATE_IOSQ: u8 = 0x01;
const ADMIN_GET_LOG_PAGE: u8 = 0x02;
const ADMIN_DELETE_IOCQ: u8 = 0x04;
const ADMIN_CREATE_IOCQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_ABORT: u8 = 0x08;
const ADMIN_SET_FEATURES: u8 = 0x09;
const ADMIN_GET_FEATURES: u8 = 0x0A;
const ADMIN_ASYNC_EVENT: u8 = 0x0C;

// NVM command opcodes.
const NVM_WRITE: u8 = 0x01;
const NVM_READ: u8 = 0x02;
const NVM_FLUSH: u8 = 0x00;

// Completion status codes.
const STS_SUCCESS: u32 = 0x0000;
const STS_INVALID_OPCODE: u32 = 0x0001;
const STS_INVALID_FIELD: u32 = 0x0002;
const STS_INVALID_QID: u32 = 0x0003;
const STS_Q_SIZE_EXCEEDED: u32 = 0x0008;
const STS_INVALID_NS: u32 = 0x000B;
const STS_LBA_OUT_OF_RANGE: u32 = 0x0080;
const STS_CAP_EXCEEDED: u32 = 0x0081;
const STS_NS_NOT_READY: u32 = 0x0085;

const CQ_ENTRY_SIZE: u64 = 16;
const SQ_ENTRY_SIZE: u64 = 64;

#[derive(Clone, Copy)]
struct Queue {
    base: u64,
    depth: u16,
    phase: bool,
    tail: u16,
    head: u16,
    deleted: bool,
}

impl Queue {
    fn new() -> Self {
        Self { base: 0, depth: 0, phase: true, tail: 0, head: 0, deleted: true }
    }
}

struct Namespace {
    image: DiskImage,
}

/// NVMe controller.
pub struct Nvme {
    cap: u64,
    vs: u32,
    intms: u32,
    intmc: u32,
    cc: u32,
    csts: u32,
    aqa: u32,
    asq: u64,
    acq: u64,

    admin_sq: Queue,
    admin_cq: Queue,
    io_sq: Vec<Queue>,
    io_cq: Vec<Queue>,

    ns1: Option<Namespace>,
    /// Admin commands pending DMA processing (one per outstanding doorbell).
    admin_pending: u16,
    io_pending: Vec<u16>,

    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Nvme {
    pub fn new() -> Self {
        Self {
            cap: (MAX_Q_DEPTH as u64 - 1) | CAP_CSS_NVM | (8 << 24),
            vs: 0x0001_0300,
            intms: 0,
            intmc: 0,
            cc: 0,
            csts: 0,
            aqa: 0,
            asq: 0,
            acq: 0,
            admin_sq: Queue::new(),
            admin_cq: Queue::new(),
            io_sq: Vec::new(),
            io_cq: Vec::new(),
            ns1: None,
            admin_pending: 0,
            io_pending: Vec::new(),
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

    /// Attach a namespace (nsid 1). Returns the previous namespace image.
    pub fn attach_namespace(&mut self, image: DiskImage) -> Option<DiskImage> {
        self.ns1.replace(Namespace { image }).map(|n| n.image)
    }

    pub fn detach_namespace(&mut self) -> Option<DiskImage> {
        self.ns1.take().map(|namespace| namespace.image)
    }

    pub fn flush_namespace(&mut self) -> Result<(), StorageError> {
        self.ns1
            .as_mut()
            .ok_or_else(|| StorageError::InvalidImage("no namespace".into()))?
            .image
            .flush()
    }

    pub fn sync_namespace(&mut self) -> Result<(), StorageError> {
        self.ns1
            .as_mut()
            .ok_or_else(|| StorageError::InvalidImage("no namespace".into()))?
            .image
            .sync()
    }

    pub fn has_pending(&self) -> bool {
        self.admin_pending != 0 || self.io_pending.iter().any(|&p| p != 0)
    }

    /// Process all pending admin and I/O commands against `mmu`.
    pub fn poll_dma(&mut self, mmu: &mut Mmu) {
        if self.admin_pending != 0 {
            self.process_admin(mmu);
            self.admin_pending = 0;
        }
        // Collect pending queue indices first to avoid aliasing
        // self.io_pending while calling &mut self methods.
        let pending: Vec<usize> = self
            .io_pending
            .iter()
            .enumerate()
            .filter(|(_, &p)| p != 0)
            .map(|(i, _)| i)
            .collect();
        for i in pending {
            self.process_io(mmu, i);
            self.io_pending[i] = 0;
        }
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

    fn read_reg(&self, off: u64) -> u64 {
        match off {
            REG_CAP => self.cap,
            REG_VS => self.vs as u64,
            REG_INTMS => self.intms as u64,
            REG_INTMC => self.intmc as u64,
            REG_CC => self.cc as u64,
            REG_CSTS => self.csts as u64,
            REG_AQA => self.aqa as u64,
            REG_ASQ => self.asq,
            REG_ACQ => self.acq,
            _ => 0,
        }
    }

    fn write_reg(&mut self, off: u64, value: u64) {
        match off {
            REG_INTMS => self.intms |= value as u32,
            REG_INTMC => self.intmc |= value as u32,
            REG_CC => self.write_cc(value as u32),
            REG_AQA => self.aqa = value as u32,
            REG_ASQ => self.asq = value,
            REG_ACQ => self.acq = value,
            _ => {}
        }
    }

    fn write_cc(&mut self, value: u32) {
        let enable = value & CC_EN;
        let was_enable = self.cc & CC_EN;
        self.cc = value;
        if enable != 0 && was_enable == 0 {
            let acqs = ((self.aqa >> 16) & 0x0FFF) as u16 + 1;
            let asqs = (self.aqa & 0x0FFF) as u16 + 1;
            self.admin_cq = Queue { base: self.acq, depth: acqs.max(2), phase: true, tail: 0, head: 0, deleted: false };
            self.admin_sq = Queue { base: self.asq, depth: asqs.max(2), phase: true, tail: 0, head: 0, deleted: false };
            self.io_sq.clear();
            self.io_cq.clear();
            self.io_pending.clear();
            self.csts |= CSTS_RDY;
        } else if enable == 0 && was_enable != 0 {
            self.csts &= !CSTS_RDY;
        }
    }

    /// Ring a submission or completion queue doorbell.
    fn doorbell(&mut self, offset: u64, value: u32) {
        let door = offset - REG_DBS;
        if door % 4 != 0 {
            return;
        }
        let index = door / 4;
        match index {
            0 => {
                self.admin_sq.tail = (value & 0xFFFF) as u16;
                self.admin_pending = self.admin_pending.wrapping_add(1);
            }
            1 => {
                self.admin_cq.head = (value & 0xFFFF) as u16;
            }
            2 => {
                // SQ1 tail.
                if !self.io_sq.is_empty() {
                    self.io_sq[0].tail = (value & 0xFFFF) as u16;
                    self.io_pending[0] = self.io_pending[0].wrapping_add(1);
                }
            }
            3 => {
                if !self.io_cq.is_empty() {
                    self.io_cq[0].head = (value & 0xFFFF) as u16;
                }
            }
            _ => {
                // Higher queue indices at stride 2: SQn at 2n, CQn at 2n+1.
                let n = (index / 2) as usize;
                if index % 2 == 0 {
                    if n > 0 && n <= self.io_sq.len() {
                        self.io_sq[n - 1].tail = (value & 0xFFFF) as u16;
                        self.io_pending[n - 1] = self.io_pending[n - 1].wrapping_add(1);
                    }
                } else if n > 0 && n <= self.io_cq.len() {
                    self.io_cq[n - 1].head = (value & 0xFFFF) as u16;
                }
            }
        }
    }

    fn read_sq(mmu: &Mmu, q: &Queue, index: u16, buf: &mut [u8; 64]) -> Result<(), StorageError> {
        Self::dma_read(mmu, q.base + (index as u64) * SQ_ENTRY_SIZE, buf)
    }

    fn write_cq(
        mmu: &mut Mmu,
        q: &Queue,
        index: u16,
        status: u32,
        sqhd: u16,
        cid: u16,
    ) -> Result<(), StorageError> {
        let mut entry = [0u8; CQ_ENTRY_SIZE as usize];
        let phase = if q.phase { 1u16 } else { 0u16 };
        // DW2 (bytes 8..12): SQHD in bits 15:0, phase tag (bit 16) and
        // status code (bits 17:31) in the upper half.
        entry[8..10].copy_from_slice(&sqhd.to_le_bytes());
        entry[10..12].copy_from_slice(&((status << 1) | phase as u32).to_le_bytes());
        // DW3 (bytes 12..16): CID in bits 31:16.
        entry[14..16].copy_from_slice(&cid.to_le_bytes());
        Self::dma_write(mmu, q.base + (index as u64) * CQ_ENTRY_SIZE, &entry)
    }

    /// Advance a completion queue's tail by one entry, flipping phase on wrap.
    fn cq_advance(q: &mut Queue) {
        let next = q.tail.wrapping_add(1);
        if next == q.depth {
            q.phase = !q.phase;
            q.tail = 0;
        } else {
            q.tail = next;
        }
    }

    fn process_admin(&mut self, mmu: &mut Mmu) {
        let mut sq_head = self.admin_sq.head;
        while sq_head != self.admin_sq.tail {
            let mut cmd = [0u8; 64];
            if Self::read_sq(mmu, &self.admin_sq, sq_head, &mut cmd).is_err() {
                break;
            }
            let cid = u16::from_le_bytes([cmd[2], cmd[3]]);
            let status = self.handle_admin(mmu, &cmd);
            let cq_index = self.admin_cq.tail;
            let ok = Self::write_cq(mmu, &self.admin_cq, cq_index, status, sq_head, cid).is_ok();
            if ok {
                Self::cq_advance(&mut self.admin_cq);
                sq_head = sq_head.wrapping_add(1) % self.admin_sq.depth;
                self.raise_irq();
            } else {
                break;
            }
        }
        self.admin_sq.head = sq_head;
    }

    fn process_io(&mut self, mmu: &mut Mmu, qidx: usize) {
        let mut sq_head = self.io_sq[qidx].head;
        while sq_head != self.io_sq[qidx].tail {
            let mut cmd = [0u8; 64];
            if Self::read_sq(mmu, &self.io_sq[qidx], sq_head, &mut cmd).is_err() {
                break;
            }
            let cid = u16::from_le_bytes([cmd[2], cmd[3]]);
            let status = self.handle_io(mmu, &cmd);
            // I/O commands complete on the CQ whose ID is stored in the SQ's
            // CREATE I/O SQ CQID field; for simplicity we track a single
            // CQ per SQ via the create-time association stored in io_cq
            // (guest maps SQ qidx+1 to CQ qidx+1 in the common case).
            let cq_idx = qidx;
            if cq_idx >= self.io_cq.len() {
                break;
            }
            let cq_index = self.io_cq[cq_idx].tail;
            let ok = Self::write_cq(mmu, &self.io_cq[cq_idx], cq_index, status, sq_head, cid).is_ok();
            if ok {
                Self::cq_advance(&mut self.io_cq[cq_idx]);
                sq_head = sq_head.wrapping_add(1) % self.io_sq[qidx].depth;
                self.raise_irq();
            } else {
                break;
            }
        }
        self.io_sq[qidx].head = sq_head;
    }

    /// Handle one admin command; returns the completion status.
    fn handle_admin(&mut self, mmu: &mut Mmu, cmd: &[u8; 64]) -> u32 {
        let opcode = cmd[0];
        // NSID at DW1.
        let nsid = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
        // PRP1 at DW2.
        let prp1 = u64::from_le_bytes(cmd[8..16].try_into().unwrap());
        // CDW10 at bytes 40..44.
        let cdw10 = u32::from_le_bytes(cmd[40..44].try_into().unwrap());
        // CDW11 at bytes 44..48.
        let cdw11 = u32::from_le_bytes(cmd[44..48].try_into().unwrap());
        // CDW12 at bytes 48..52.
        let cdw12 = u32::from_le_bytes(cmd[48..52].try_into().unwrap());

        match opcode {
            ADMIN_CREATE_IOCQ => {
                let qid = (cdw10 & 0xFFFF) as usize;
                let qsize = ((cdw10 >> 16) & 0xFFFF) as u16 + 1;
                if qid == 0 || qid > 4096 {
                    return STS_INVALID_QID;
                }
                if qsize > MAX_Q_DEPTH {
                    return STS_Q_SIZE_EXCEEDED;
                }
                // PC bit in CDW11 bit 0.
                while self.io_cq.len() < qid {
                    self.io_cq.push(Queue::new());
                }
                self.io_cq[qid - 1] = Queue {
                    base: prp1,
                    depth: qsize,
                    phase: true,
                    tail: 0,
                    head: 0,
                    deleted: false,
                };
                STS_SUCCESS
            }
            ADMIN_CREATE_IOSQ => {
                let qid = (cdw10 & 0xFFFF) as usize;
                let qsize = ((cdw10 >> 16) & 0xFFFF) as u16 + 1;
                // CQID at CDW12 bits 15:0.
                let cqid = (cdw12 & 0xFFFF) as usize;
                if qid == 0 || qid > 4096 || cqid == 0 || cqid > 4096 {
                    return STS_INVALID_QID;
                }
                if qsize > MAX_Q_DEPTH {
                    return STS_Q_SIZE_EXCEEDED;
                }
                if cqid > self.io_cq.len() {
                    return STS_INVALID_QID;
                }
                while self.io_sq.len() < qid {
                    self.io_sq.push(Queue::new());
                    self.io_pending.push(0);
                }
                self.io_sq[qid - 1] = Queue {
                    base: prp1,
                    depth: qsize,
                    phase: true,
                    tail: 0,
                    head: 0,
                    deleted: false,
                };
                STS_SUCCESS
            }
            ADMIN_DELETE_IOCQ => {
                let qid = (cdw10 & 0xFFFF) as usize;
                if qid == 0 || qid > self.io_cq.len() {
                    return STS_INVALID_QID;
                }
                self.io_cq[qid - 1].deleted = true;
                STS_SUCCESS
            }
            ADMIN_DELETE_IOSQ => {
                let qid = (cdw10 & 0xFFFF) as usize;
                if qid == 0 || qid > self.io_sq.len() {
                    return STS_INVALID_QID;
                }
                self.io_sq[qid - 1].deleted = true;
                STS_SUCCESS
            }
            ADMIN_IDENTIFY => {
                if nsid > 1 {
                    return STS_INVALID_NS;
                }
                let cns = (cdw10 & 0xFF) as u8;
                match cns {
                    0 => self.identify_namespace(mmu, prp1),
                    1 => self.identify_controller(mmu, prp1),
                    2 => {
                        let mut list = [0u8; 4096];
                        list[0..4].copy_from_slice(&1u32.to_le_bytes());
                        if Self::dma_write(mmu, prp1, &list).is_err() {
                            return STS_CAP_EXCEEDED;
                        }
                        STS_SUCCESS
                    }
                    _ => STS_INVALID_FIELD,
                }
            }
            ADMIN_GET_FEATURES | ADMIN_SET_FEATURES => {
                let fid = (cdw10 & 0xFF) as u8;
                match fid {
                    0x01 | 0x07 => {
                        // Number of queues: at most 1 of each.
                        let mut buf = [0u8; 4];
                        buf[0] = 1;
                        buf[2] = 1;
                        if opcode == ADMIN_GET_FEATURES {
                            let _ = Self::dma_write(mmu, prp1, &buf);
                        }
                        STS_SUCCESS
                    }
                    0x02 | 0x04 => STS_SUCCESS,
                    _ => STS_INVALID_FIELD,
                }
            }
            ADMIN_GET_LOG_PAGE => {
                let buf = [0u8; 512];
                let _ = Self::dma_write(mmu, prp1, &buf);
                STS_SUCCESS
            }
            ADMIN_ABORT | ADMIN_ASYNC_EVENT => {
                let _ = cdw11;
                STS_SUCCESS
            }
            _ => STS_INVALID_OPCODE,
        }
    }

    fn identify_namespace(&mut self, mmu: &mut Mmu, prp1: u64) -> u32 {
        let Some(ns) = &self.ns1 else { return STS_INVALID_NS };
        let sectors = ns.image.sector_count();
        let mut id = [0u8; 4096];
        id[0..8].copy_from_slice(&sectors.to_le_bytes()); // NSZE
        id[8..16].copy_from_slice(&sectors.to_le_bytes()); // NUSE
        id[26] = 0; // FLBAS
        id[27] = 0; // NLBAF = 1 format - 1
        id[129] = 9; // LBADS = 512 bytes
        if Self::dma_write(mmu, prp1, &id).is_err() {
            return STS_CAP_EXCEEDED;
        }
        STS_SUCCESS
    }

    fn identify_controller(&mut self, mmu: &mut Mmu, prp1: u64) -> u32 {
        let mut c = [0u8; 4096];
        c[0..2].copy_from_slice(&NVME_VENDOR_ID.to_le_bytes());
        c[2..4].copy_from_slice(&NVME_VENDOR_ID.to_le_bytes());
        let sn = b"SYNOSVM00001";
        c[4..4 + sn.len()].copy_from_slice(sn);
        let mn = b"SynOS NVMe Virtual Disk";
        c[24..24 + mn.len()].copy_from_slice(mn);
        c[64..68].copy_from_slice(b"0.1\0");
        c[512] = 6; // SQES min
        c[513] = 6; // SQES max
        c[514] = 4; // CQES min
        c[515] = 4; // CQES max
        c[516..520].copy_from_slice(&1u32.to_le_bytes()); // NN
        if Self::dma_write(mmu, prp1, &c).is_err() {
            return STS_CAP_EXCEEDED;
        }
        STS_SUCCESS
    }

    /// Handle one I/O command; returns the completion status.
    fn handle_io(&mut self, mmu: &mut Mmu, cmd: &[u8; 64]) -> u32 {
        let opcode = cmd[0];
        let nsid = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
        if nsid != 1 {
            return STS_INVALID_NS;
        }
        let prp1 = u64::from_le_bytes(cmd[8..16].try_into().unwrap());
        // CDW10 = SLBA lower 32 bits; CDW11 bits 15:0 = SLBA upper,
        // bits 31:16 = NLB.
        let cdw10 = u32::from_le_bytes(cmd[40..44].try_into().unwrap());
        let cdw11 = u32::from_le_bytes(cmd[44..48].try_into().unwrap());
        let slba = (cdw10 as u64) | (((cdw11 & 0xFFFF) as u64) << 32);
        let nlb = ((cdw11 >> 16) & 0xFFFF) as u64 + 1;

        match opcode {
            NVM_FLUSH => STS_SUCCESS,
            NVM_READ | NVM_WRITE => {
                let Some(ns) = self.ns1.as_mut() else { return STS_NS_NOT_READY };
                let sectors = ns.image.sector_count();
                if slba.saturating_add(nlb) > sectors {
                    return STS_LBA_OUT_OF_RANGE;
                }
                let total = (nlb as usize) * 512;
                let mut buf = vec![0u8; total];
                if opcode == NVM_READ {
                    for i in 0..nlb as usize {
                        let mut sector = [0u8; 512];
                        if ns.image.read_sector(slba + i as u64, &mut sector).is_err() {
                            return STS_LBA_OUT_OF_RANGE;
                        }
                        buf[i * 512..i * 512 + 512].copy_from_slice(&sector);
                    }
                    if Self::dma_write(mmu, prp1, &buf).is_err() {
                        return STS_CAP_EXCEEDED;
                    }
                } else {
                    if Self::dma_read(mmu, prp1, &mut buf).is_err() {
                        return STS_CAP_EXCEEDED;
                    }
                    for i in 0..nlb as usize {
                        let sector: [u8; 512] = buf[i * 512..i * 512 + 512].try_into().unwrap();
                        if ns.image.write_sector(slba + i as u64, &sector).is_err() {
                            return STS_LBA_OUT_OF_RANGE;
                        }
                    }
                }
                STS_SUCCESS
            }
            _ => STS_INVALID_OPCODE,
        }
    }

    fn raise_irq(&mut self) {
        if self.irq_vector != 0 {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
            }
        }
    }
}

impl Default for Nvme {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Nvme {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let off = addr & 0x1FFF;
        if off < REG_DBS {
            match size {
                4 => Ok(self.read_reg(off) & 0xFFFF_FFFF),
                8 => Ok(self.read_reg(off)),
                _ => Err(DeviceError::UnsupportedSize),
            }
        } else if off >= REG_DBS && size == 4 {
            // Doorbell reads return 0 (writes only).
            let _ = off;
            Ok(0)
        } else {
            Err(DeviceError::UnsupportedSize)
        }
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        let off = addr & 0x1FFF;
        if off < REG_DBS {
            if size != 4 && size != 8 {
                return Err(DeviceError::UnsupportedSize);
            }
            self.write_reg(off, value);
            Ok(())
        } else if off >= REG_DBS && size == 4 {
            self.doorbell(off, value as u32);
            Ok(())
        } else {
            Err(DeviceError::UnsupportedSize)
        }
    }

    fn reset(&mut self) {
        let apic = self.apic.take();
        let v = self.irq_vector;
        let ns = self.ns1.take();
        *self = Self::new();
        self.apic = apic;
        self.irq_vector = v;
        self.ns1 = ns;
    }
}

impl Device for Rc<RefCell<Nvme>> {
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

    fn disk(name: &str) -> DiskImage {
        let path = std::env::temp_dir().join(format!("synos-nvme-{name}-{}", std::process::id()));
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();
        DiskImage::open(path).unwrap()
    }

    fn command(opcode: u8, buffer: u64, sector: u64, sectors: u16) -> [u8; 64] {
        let mut cmd = [0u8; 64];
        cmd[0] = opcode;
        cmd[4..8].copy_from_slice(&1u32.to_le_bytes());
        cmd[8..16].copy_from_slice(&buffer.to_le_bytes());
        cmd[40..44].copy_from_slice(&(sector as u32).to_le_bytes());
        cmd[44..48].copy_from_slice(&(((u32::from(sectors) - 1) << 16) | (sector >> 32) as u32).to_le_bytes());
        cmd
    }

    #[test]
    fn controller_registers_identify_and_io_round_trip() {
        let mut nvme = Nvme::new();
        nvme.attach_namespace(disk("round-trip"));
        assert_eq!(Device::read(&nvme, REG_VS, 4).unwrap(), 0x0001_0300);
        assert!(nvme.cap & CAP_CSS_NVM != 0);

        let mut mmu = Mmu::new(0x20_000);
        let buffer = 0x2000;
        let payload = [0x5Au8; 512];
        mmu.write_phys(buffer, &payload).unwrap();
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_WRITE, buffer, 1, 1)), STS_SUCCESS);
        mmu.write_phys(buffer, &[0; 512]).unwrap();
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_READ, buffer, 1, 1)), STS_SUCCESS);
        assert_eq!(mmu.read_phys(buffer, 512).unwrap(), payload);

        let identify_buffer = 0x3000;
        let mut identify = command(ADMIN_IDENTIFY, identify_buffer, 0, 1);
        identify[40..44].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(nvme.handle_admin(&mut mmu, &identify), STS_SUCCESS);
        assert_eq!(u64::from_le_bytes(mmu.read_phys(identify_buffer, 8).unwrap().try_into().unwrap()), 8);
    }

    #[test]
    fn invalid_namespace_and_bounds_fail_cleanly() {
        let mut nvme = Nvme::new();
        let mut mmu = Mmu::new(0x10_000);
        let mut cmd = command(NVM_READ, 0x2000, 0, 1);
        cmd[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(nvme.handle_io(&mut mmu, &cmd), STS_INVALID_NS);
        nvme.attach_namespace(disk("bounds"));
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_READ, 0x2000, 8, 1)), STS_LBA_OUT_OF_RANGE);
        assert_eq!(Device::read(&nvme, REG_VS, 2), Err(DeviceError::UnsupportedSize));
        nvme.reset();
        assert_eq!(nvme.ns1.as_ref().map(|n| n.image.sector_count()), Some(8));
    }
}
