use crate::{Access, AddressRange, Error, NodeId, PAGE_SIZE, PageFault};

pub const SYNOS_DSM_ETHERTYPE: u16 = 0x88b5;
pub const FRAME_DATA_BYTES: usize = 1400;
pub const PAGE_FRAGMENT_COUNT: usize = 3;
pub const DEFAULT_COHERENCE_PAGES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    PageRequest = 1,
    PageData = 2,
    Invalidate = 3,
    InvalidateAck = 4,
    LeaseRequest = 5,
    LeaseGrant = 6,
}

impl MessageKind {
    fn from_raw(raw: u8) -> Result<Self, Error> {
        match raw {
            1 => Ok(Self::PageRequest),
            2 => Ok(Self::PageData),
            3 => Ok(Self::Invalidate),
            4 => Ok(Self::InvalidateAck),
            5 => Ok(Self::LeaseRequest),
            6 => Ok(Self::LeaseGrant),
            _ => Err(Error::CorruptPacket),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmHeader {
    pub kind: MessageKind,
    pub source: NodeId,
    pub destination: NodeId,
    pub sequence: u32,
    pub page_address: u64,
    pub lease_epoch: u32,
    pub fragment: u16,
    pub fragment_count: u16,
}

#[derive(Clone, Copy)]
pub struct DsmPacket {
    pub header: DsmHeader,
    payload: [u8; FRAME_DATA_BYTES],
    payload_length: u16,
}

impl DsmPacket {
    const HEADER_BYTES: usize = 32;
    const VERSION: u8 = 1;

    pub fn new(header: DsmHeader, payload: &[u8]) -> Result<Self, Error> {
        if payload.len() > FRAME_DATA_BYTES
            || header.fragment_count == 0
            || header.fragment >= header.fragment_count
            || header.page_address % PAGE_SIZE != 0
        {
            return Err(Error::CorruptPacket)
        }
        let mut stored = [0; FRAME_DATA_BYTES];
        stored[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            header,
            payload: stored,
            payload_length: payload.len() as u16,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_length as usize]
    }

    /// Encodes the SynOS DSM payload placed directly after an Ethernet header.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, Error> {
        let length = Self::HEADER_BYTES + self.payload_length as usize;
        if output.len() < length {
            return Err(Error::Capacity)
        }
        output[..length].fill(0);
        output[0..2].copy_from_slice(&SYNOS_DSM_ETHERTYPE.to_be_bytes());
        output[2] = Self::VERSION;
        output[3] = self.header.kind as u8;
        output[4..8].copy_from_slice(&self.header.source.raw().to_be_bytes());
        output[8..12].copy_from_slice(&self.header.destination.raw().to_be_bytes());
        output[12..16].copy_from_slice(&self.header.sequence.to_be_bytes());
        output[16..24].copy_from_slice(&self.header.page_address.to_be_bytes());
        output[24..28].copy_from_slice(&self.header.lease_epoch.to_be_bytes());
        output[28..30].copy_from_slice(&self.header.fragment.to_be_bytes());
        output[30..32].copy_from_slice(&self.header.fragment_count.to_be_bytes());
        output[Self::HEADER_BYTES..length].copy_from_slice(self.payload());
        Ok(length)
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() < Self::HEADER_BYTES
            || u16::from_be_bytes([input[0], input[1]]) != SYNOS_DSM_ETHERTYPE
            || input[2] != Self::VERSION
        {
            return Err(Error::CorruptPacket)
        }
        let source = NodeId::new(read_u32(input, 4)).ok_or(Error::CorruptPacket)?;
        let destination = NodeId::new(read_u32(input, 8)).ok_or(Error::CorruptPacket)?;
        let header = DsmHeader {
            kind: MessageKind::from_raw(input[3])?,
            source,
            destination,
            sequence: read_u32(input, 12),
            page_address: read_u64(input, 16),
            lease_epoch: read_u32(input, 24),
            fragment: read_u16(input, 28),
            fragment_count: read_u16(input, 30),
        };
        Self::new(header, &input[Self::HEADER_BYTES..])
    }

    pub fn page_fragment(
        source: NodeId,
        destination: NodeId,
        sequence: u32,
        page_address: u64,
        lease_epoch: u32,
        fragment: usize,
        page: &[u8; PAGE_SIZE as usize],
    ) -> Result<Self, Error> {
        if fragment >= PAGE_FRAGMENT_COUNT {
            return Err(Error::InvalidRange)
        }
        let start = fragment * FRAME_DATA_BYTES;
        let end = core::cmp::min(start + FRAME_DATA_BYTES, page.len());
        Self::new(
            DsmHeader {
                kind: MessageKind::PageData,
                source,
                destination,
                sequence,
                page_address,
                lease_epoch,
                fragment: fragment as u16,
                fragment_count: PAGE_FRAGMENT_COUNT as u16,
            },
            &page[start..end],
        )
    }
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([input[offset], input[offset + 1]])
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}

pub struct PageAssembler {
    page_address: u64,
    sequence: u32,
    received: u8,
    bytes: [u8; PAGE_SIZE as usize],
}

impl PageAssembler {
    pub const fn new(page_address: u64, sequence: u32) -> Self {
        Self {
            page_address,
            sequence,
            received: 0,
            bytes: [0; PAGE_SIZE as usize],
        }
    }

    pub fn push(&mut self, packet: &DsmPacket) -> Result<bool, Error> {
        let fragment = packet.header.fragment as usize;
        if packet.header.kind != MessageKind::PageData
            || packet.header.page_address != self.page_address
            || packet.header.sequence != self.sequence
            || packet.header.fragment_count as usize != PAGE_FRAGMENT_COUNT
            || fragment >= PAGE_FRAGMENT_COUNT
        {
            return Err(Error::CorruptPacket)
        }
        let start = fragment * FRAME_DATA_BYTES;
        let end = start
            .checked_add(packet.payload().len())
            .ok_or(Error::CorruptPacket)?;
        if end > self.bytes.len()
            || (fragment + 1 < PAGE_FRAGMENT_COUNT
                && packet.payload().len() != FRAME_DATA_BYTES)
        {
            return Err(Error::CorruptPacket)
        }
        self.bytes[start..end].copy_from_slice(packet.payload());
        self.received |= 1 << fragment;
        Ok(self.received == (1 << PAGE_FRAGMENT_COUNT) - 1)
    }

    pub fn page(&self) -> Option<&[u8; PAGE_SIZE as usize]> {
        if self.received == (1 << PAGE_FRAGMENT_COUNT) - 1 {
            Some(&self.bytes)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DlmLeaseMode {
    ProtectedRead,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftwareDlmLease {
    pub owner: NodeId,
    pub mode: DlmLeaseMode,
    pub epoch: u32,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemotePageAuthority {
    pub subject: NodeId,
    pub range: AddressRange,
    pub read: bool,
    pub write: bool,
    pub lease_epoch: u32,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoherenceAction {
    MapLocal {
        writable: bool,
    },
    AcquireDlmLease {
        page_address: u64,
        mode: DlmLeaseMode,
    },
    FetchPage {
        page_address: u64,
        from: NodeId,
        lease: SoftwareDlmLease,
    },
    Invalidate {
        page_address: u64,
        nodes: u64,
        lease: SoftwareDlmLease,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageState {
    Invalid,
    Shared,
    Exclusive,
}

#[derive(Clone, Copy)]
struct CoherenceEntry {
    page_address: u64,
    state: PageState,
    owner: Option<NodeId>,
    sharers: u64,
    lease: Option<SoftwareDlmLease>,
}

impl CoherenceEntry {
    const EMPTY: Self = Self {
        page_address: u64::MAX,
        state: PageState::Invalid,
        owner: None,
        sharers: 0,
        lease: None,
    };
}

/// Directory coherence controlled by per-page software DLM leases.
///
/// A page is mapped writable only for the live exclusive lease holder.
/// Read leases produce shared mappings. Writers invalidate every other sharer
/// before the mapping becomes writable.
pub struct CoherenceDirectory<const CAPACITY: usize = DEFAULT_COHERENCE_PAGES> {
    pages: [CoherenceEntry; CAPACITY],
}

impl<const CAPACITY: usize> CoherenceDirectory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            pages: [CoherenceEntry::EMPTY; CAPACITY],
        }
    }

    pub fn begin_fault(
        &mut self,
        requester: NodeId,
        fault: PageFault,
        now_us: u64,
    ) -> Result<CoherenceAction, Error> {
        if fault.reserved_bit || fault.access == Access::Execute {
            return Err(Error::InvalidAddress)
        }
        let page_address = fault.page_address();
        let entry = self.entry(page_address)?;
        if let Some(lease) = entry.lease
            && now_us >= lease.expires_at_us
        {
            entry.lease = None;
            entry.state = PageState::Invalid;
            entry.owner = None;
            entry.sharers = 0
        }

        let requested_mode = if fault.access == Access::Write {
            DlmLeaseMode::Exclusive
        } else {
            DlmLeaseMode::ProtectedRead
        };
        let valid_local_lease = entry
            .lease
            .is_some_and(|lease| lease.owner == requester && now_us < lease.expires_at_us);
        if !valid_local_lease {
            return Ok(CoherenceAction::AcquireDlmLease {
                page_address,
                mode: requested_mode,
            })
        }

        if fault.access == Access::Write {
            let requester_bit = node_bit(requester)?;
            let others = entry.sharers & !requester_bit;
            if others != 0 {
                return Ok(CoherenceAction::Invalidate {
                    page_address,
                    nodes: others,
                    lease: entry.lease.expect("checked lease"),
                })
            }
            if entry.state == PageState::Exclusive && entry.owner == Some(requester) {
                return Ok(CoherenceAction::MapLocal { writable: true })
            }
        } else if entry.sharers & node_bit(requester)? != 0 {
            return Ok(CoherenceAction::MapLocal { writable: false })
        }

        let source = entry.owner.or_else(|| first_node(entry.sharers));
        match source {
            Some(from) if from != requester => Ok(CoherenceAction::FetchPage {
                page_address,
                from,
                lease: entry.lease.expect("checked lease"),
            }),
            _ => Ok(CoherenceAction::MapLocal {
                writable: fault.access == Access::Write,
            }),
        }
    }

    /// Authorize a cross-node page fault before touching DSM state.
    ///
    /// The cryptographic token layer creates this short-lived authority. The
    /// DLM epoch check fences revoked or stale tokens cluster-wide.
    pub fn begin_remote_fault(
        &mut self,
        requester: NodeId,
        fault: PageFault,
        authority: RemotePageAuthority,
        now_us: u64,
    ) -> Result<CoherenceAction, Error> {
        let access_allowed = match fault.access {
            Access::Read => authority.read,
            Access::Write => authority.write,
            Access::Execute => false,
        };
        if authority.subject != requester
            || !authority.range.contains(fault.page_address())
            || !access_allowed
            || now_us >= authority.expires_at_us
        {
            return Err(Error::NotOwner)
        }
        let entry = self.entry(fault.page_address())?;
        let lease = entry.lease.ok_or(Error::ExpiredLease)?;
        if lease.owner != requester
            || lease.epoch != authority.lease_epoch
            || now_us >= lease.expires_at_us
        {
            return Err(Error::ExpiredLease)
        }
        self.begin_fault(requester, fault, now_us)
    }

    pub fn grant_lease(
        &mut self,
        page_address: u64,
        lease: SoftwareDlmLease,
    ) -> Result<(), Error> {
        if page_address % PAGE_SIZE != 0 || lease.expires_at_us == 0 {
            return Err(Error::InvalidRange)
        }
        let entry = self.entry(page_address)?;
        entry.lease = Some(lease);
        Ok(())
    }

    pub fn page_arrived(
        &mut self,
        page_address: u64,
        node: NodeId,
        writable: bool,
    ) -> Result<(), Error> {
        let entry = self.entry(page_address)?;
        if writable {
            entry.state = PageState::Exclusive;
            entry.owner = Some(node);
            entry.sharers = 0
        } else {
            entry.state = PageState::Shared;
            entry.sharers |= node_bit(node)?
        }
        Ok(())
    }

    pub fn invalidation_complete(
        &mut self,
        page_address: u64,
        writer: NodeId,
    ) -> Result<(), Error> {
        let entry = self.entry(page_address)?;
        let lease = entry.lease.ok_or(Error::ExpiredLease)?;
        if lease.owner != writer || lease.mode != DlmLeaseMode::Exclusive {
            return Err(Error::NotOwner)
        }
        entry.state = PageState::Exclusive;
        entry.owner = Some(writer);
        entry.sharers = 0;
        Ok(())
    }

    pub fn fail_node(&mut self, node: NodeId) -> Result<usize, Error> {
        let bit = node_bit(node)?;
        let mut changed = 0;
        for entry in &mut self.pages {
            if entry.page_address == u64::MAX {
                continue
            }
            let affected = entry.owner == Some(node)
                || entry.sharers & bit != 0
                || entry.lease.is_some_and(|lease| lease.owner == node);
            if !affected {
                continue
            }
            entry.sharers &= !bit;
            if entry.owner == Some(node) {
                entry.owner = None;
                entry.state = if entry.sharers == 0 {
                    PageState::Invalid
                } else {
                    PageState::Shared
                }
            }
            if entry.lease.is_some_and(|lease| lease.owner == node) {
                entry.lease = None
            }
            changed += 1
        }
        Ok(changed)
    }

    fn entry(&mut self, page_address: u64) -> Result<&mut CoherenceEntry, Error> {
        if let Some(index) = self
            .pages
            .iter()
            .position(|entry| entry.page_address == page_address)
        {
            return Ok(&mut self.pages[index])
        }
        let entry = self
            .pages
            .iter_mut()
            .find(|entry| entry.page_address == u64::MAX)
            .ok_or(Error::Capacity)?;
        entry.page_address = page_address;
        Ok(entry)
    }
}

impl<const CAPACITY: usize> Default for CoherenceDirectory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn node_bit(node: NodeId) -> Result<u64, Error> {
    if node.raw() > 64 {
        Err(Error::Capacity)
    } else {
        Ok(1 << (node.raw() - 1))
    }
}

fn first_node(nodes: u64) -> Option<NodeId> {
    NodeId::new(nodes.trailing_zeros() + 1).filter(|_| nodes != 0)
}
