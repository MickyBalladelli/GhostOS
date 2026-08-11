//! Bounded, redacted continuous profiling primitives.
//!
//! Producers record already-symbolized stable symbol IDs. No instruction
//! addresses, paths, arguments, payloads, host names, or identities enter the
//! ring. A collector drains the ring into a deterministic folded-stack archive
//! tagged with a source revision and a redacted host fingerprint.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use synos_system_model::ContentId;

pub const MAX_PROFILE_FRAMES: usize = 16;
pub const GLOBAL_PROFILE_CAPACITY: usize = 256;
pub const MAX_PROFILE_STACKS: usize = 128;
pub const PROFILE_MAGIC: &[u8; 8] = b"SNPROF01";
pub const PROFILE_VERSION: u16 = 1;
pub const PROFILE_HEADER_BYTES: usize = 112;
pub const PROFILE_STACK_RECORD_BYTES: usize = 208;
pub const PROFILE_CHECKSUM_OFFSET: usize = 104;

const PROFILE_SLOT_BUSY: u64 = 1 << 63;
const EMPTY_FRAME: ProfileFrame = ProfileFrame { symbol: 0, offset: 0 };

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProfileDomain {
    Boot = 1,
    Ipc = 2,
    Scheduler = 3,
    SynFs = 4,
    Networking = 5,
    PackageActivation = 6,
    CompilerBuild = 7,
    VmExecution = 8,
    ClientRpc = 9,
}

impl ProfileDomain {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Boot),
            2 => Some(Self::Ipc),
            3 => Some(Self::Scheduler),
            4 => Some(Self::SynFs),
            5 => Some(Self::Networking),
            6 => Some(Self::PackageActivation),
            7 => Some(Self::CompilerBuild),
            8 => Some(Self::VmExecution),
            9 => Some(Self::ClientRpc),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Ipc => "ipc",
            Self::Scheduler => "scheduler",
            Self::SynFs => "synfs",
            Self::Networking => "networking",
            Self::PackageActivation => "package-activation",
            Self::CompilerBuild => "compiler-build",
            Self::VmExecution => "vm-execution",
            Self::ClientRpc => "client-rpc",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileFrame {
    /// Stable generated symbol ID. This is not a code address.
    pub symbol: u64,
    /// Offset within the symbol, retained for folded-stack comparisons.
    pub offset: u32,
}

impl ProfileFrame {
    pub const fn new(symbol: u64, offset: u32) -> Option<Self> {
        if symbol == 0 {
            None
        } else {
            Some(Self { symbol, offset })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileSample {
    pub timestamp: u64,
    pub cpu: u32,
    pub domain: ProfileDomain,
    frames: [ProfileFrame; MAX_PROFILE_FRAMES],
    depth: u8,
}

impl ProfileSample {
    pub const fn single(domain: ProfileDomain, timestamp: u64, cpu: u32, symbol: u64) -> Self {
        let mut sample = Self {
            timestamp,
            cpu,
            domain,
            frames: [EMPTY_FRAME; MAX_PROFILE_FRAMES],
            depth: 0,
        };
        if symbol != 0 {
            sample.frames[0] = ProfileFrame { symbol, offset: 0 };
            sample.depth = 1;
        }
        sample
    }

    pub const fn with_frame(mut self, symbol: u64, offset: u32) -> Self {
        if symbol != 0 && (self.depth as usize) < MAX_PROFILE_FRAMES {
            self.frames[self.depth as usize] = ProfileFrame { symbol, offset };
            self.depth += 1;
        }
        self
    }

    pub const fn depth(self) -> usize {
        self.depth as usize
    }

    pub fn frames(&self) -> impl Iterator<Item = ProfileFrame> + '_ {
        self.frames[..self.depth as usize].iter().copied()
    }

    pub fn frame(&self, index: usize) -> Option<ProfileFrame> {
        self.frames.get(index).copied().filter(|frame| frame.symbol != 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileMetadata {
    pub revision: ContentId,
    pub host_id: [u8; 16],
    pub sample_period_us: u64,
    pub started_at: u64,
    pub ended_at: u64,
}

impl ProfileMetadata {
    pub fn new(
        revision: ContentId,
        host_material: &[u8],
        sample_period_us: u64,
        started_at: u64,
    ) -> Self {
        Self {
            revision,
            host_id: redacted_host_id(host_material),
            sample_period_us: sample_period_us.max(1),
            started_at,
            ended_at: 0,
        }
    }

    pub const fn finish(mut self, ended_at: u64) -> Self {
        self.ended_at = ended_at;
        self
    }
}

/// Hash host material before it can enter a profile. Callers should pass
/// stable hardware facts, never a hostname, network address, or user data.
pub fn redacted_host_id(host_material: &[u8]) -> [u8; 16] {
    let mut material = [0u8; 64];
    let prefix = b"synos-profile-host-v1";
    let prefix_len = prefix.len().min(material.len());
    material[..prefix_len].copy_from_slice(&prefix[..prefix_len]);
    let available = material.len().saturating_sub(prefix_len);
    let length = host_material.len().min(available);
    material[prefix_len..prefix_len + length].copy_from_slice(&host_material[..length]);
    let digest = ContentId::hash(&material[..prefix_len + length]);
    let mut result = [0; 16];
    result.copy_from_slice(&digest.as_bytes()[..16]);
    result
}

struct AtomicProfileSample {
    timestamp: AtomicU64,
    cpu: AtomicU32,
    domain: AtomicU8,
    depth: AtomicU8,
    symbols: [AtomicU64; MAX_PROFILE_FRAMES],
    offsets: [AtomicU32; MAX_PROFILE_FRAMES],
}

impl AtomicProfileSample {
    const fn new() -> Self {
        Self {
            timestamp: AtomicU64::new(0),
            cpu: AtomicU32::new(0),
            domain: AtomicU8::new(0),
            depth: AtomicU8::new(0),
            symbols: [const { AtomicU64::new(0) }; MAX_PROFILE_FRAMES],
            offsets: [const { AtomicU32::new(0) }; MAX_PROFILE_FRAMES],
        }
    }

    fn write(&self, sample: ProfileSample) {
        self.timestamp.store(sample.timestamp, Ordering::Relaxed);
        self.cpu.store(sample.cpu, Ordering::Relaxed);
        self.domain.store(sample.domain as u8, Ordering::Relaxed);
        self.depth.store(sample.depth, Ordering::Relaxed);
        for index in 0..MAX_PROFILE_FRAMES {
            self.symbols[index].store(sample.frames[index].symbol, Ordering::Relaxed);
            self.offsets[index].store(sample.frames[index].offset, Ordering::Relaxed);
        }
    }

    fn read(&self) -> Option<ProfileSample> {
        let domain = ProfileDomain::from_raw(self.domain.load(Ordering::Relaxed))?;
        let depth = (self.depth.load(Ordering::Relaxed) as usize).min(MAX_PROFILE_FRAMES);
        if depth == 0 {
            return None;
        }
        let mut frames = [EMPTY_FRAME; MAX_PROFILE_FRAMES];
        for index in 0..MAX_PROFILE_FRAMES {
            frames[index] = ProfileFrame {
                symbol: self.symbols[index].load(Ordering::Relaxed),
                offset: self.offsets[index].load(Ordering::Relaxed),
            };
        }
        Some(ProfileSample {
            timestamp: self.timestamp.load(Ordering::Relaxed),
            cpu: self.cpu.load(Ordering::Relaxed),
            domain,
            frames,
            depth: depth as u8,
        })
    }
}

struct ProfileSlot {
    published: AtomicU64,
    sample: AtomicProfileSample,
}

impl ProfileSlot {
    const fn new() -> Self {
        Self {
            published: AtomicU64::new(0),
            sample: AtomicProfileSample::new(),
        }
    }
}

/// Lock-free overwrite-oldest sample ring. A full ring drops old samples and
/// never blocks the profiled path.
pub struct ProfileRing<const CAPACITY: usize> {
    write_position: AtomicU64,
    read_position: AtomicU64,
    dropped: AtomicU64,
    slots: [ProfileSlot; CAPACITY],
}

impl<const CAPACITY: usize> ProfileRing<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 2);
        Self {
            write_position: AtomicU64::new(0),
            read_position: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            slots: [const { ProfileSlot::new() }; CAPACITY],
        }
    }

    pub fn push(&self, sample: ProfileSample) {
        if sample.depth == 0 {
            return
        }
        let ticket = self.write_position.fetch_add(1, Ordering::AcqRel);
        let minimum = ticket.saturating_add(1).saturating_sub(CAPACITY as u64);
        let mut read = self.read_position.load(Ordering::Acquire);
        while read < minimum {
            match self.read_position.compare_exchange_weak(
                read,
                minimum,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.dropped.fetch_add(minimum - read, Ordering::Relaxed);
                    break
                }
                Err(observed) => read = observed,
            }
        }
        let slot = &self.slots[ticket as usize % CAPACITY];
        slot.published.store(ticket | PROFILE_SLOT_BUSY, Ordering::Release);
        slot.sample.write(sample);
        slot.published.store(ticket, Ordering::Release)
    }

    pub fn try_pop(&self) -> Option<ProfileSample> {
        loop {
            let position = self.read_position.load(Ordering::Acquire);
            if position >= self.write_position.load(Ordering::Acquire) {
                return None;
            }
            let slot = &self.slots[position as usize % CAPACITY];
            let published = slot.published.load(Ordering::Acquire);
            if published & PROFILE_SLOT_BUSY != 0 {
                return None;
            }
            if published != position {
                if published > position {
                    let _ = self.read_position.compare_exchange_weak(
                        position,
                        published,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    continue
                }
                return None;
            }
            if self
                .read_position
                .compare_exchange_weak(position, position + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue
            }
            let sample = slot.sample.read();
            if slot.published.load(Ordering::Acquire) == position {
                return sample
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<const CAPACITY: usize> Default for ProfileRing<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub static SYSTEM_PROFILE: ProfileRing<GLOBAL_PROFILE_CAPACITY> = ProfileRing::new();

pub fn record_profile_sample(sample: ProfileSample) {
    SYSTEM_PROFILE.push(sample)
}

pub fn drain_profile(destination: &mut [Option<ProfileSample>]) -> usize {
    let mut count = 0;
    while count < destination.len() {
        let Some(sample) = SYSTEM_PROFILE.try_pop() else { break };
        destination[count] = Some(sample);
        count += 1;
    }
    count
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileStack {
    pub domain: ProfileDomain,
    frames: [ProfileFrame; MAX_PROFILE_FRAMES],
    depth: u8,
    pub samples: u64,
}

impl ProfileStack {
    fn from_sample(sample: ProfileSample) -> Self {
        Self {
            domain: sample.domain,
            frames: sample.frames,
            depth: sample.depth,
            samples: 1,
        }
    }

    pub const fn depth(self) -> usize {
        self.depth as usize
    }

    pub fn frames(&self) -> impl Iterator<Item = ProfileFrame> + '_ {
        self.frames[..self.depth as usize].iter().copied()
    }

    fn matches(&self, sample: ProfileSample) -> bool {
        self.domain == sample.domain
            && self.depth == sample.depth
            && self.frames[..self.depth as usize] == sample.frames[..sample.depth as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    Capacity,
    BufferTooSmall { required: usize },
    InvalidExport,
}

/// Fold samples into bounded, deterministic stacks for export.
pub struct ProfileAggregator<const CAPACITY: usize = MAX_PROFILE_STACKS> {
    pub metadata: ProfileMetadata,
    stacks: [Option<ProfileStack>; CAPACITY],
    sample_count: u64,
    dropped: u64,
    observed_ring_dropped: u64,
}

impl<const CAPACITY: usize> ProfileAggregator<CAPACITY> {
    pub const fn new(metadata: ProfileMetadata) -> Self {
        Self {
            metadata,
            stacks: [None; CAPACITY],
            sample_count: 0,
            dropped: 0,
            observed_ring_dropped: 0,
        }
    }

    pub fn ingest(&mut self, sample: ProfileSample) -> Result<(), ProfileError> {
        if sample.depth == 0 {
            return Err(ProfileError::InvalidExport);
        }
        self.sample_count = self.sample_count.saturating_add(1);
        if let Some(stack) = self
            .stacks
            .iter_mut()
            .flatten()
            .find(|stack| stack.matches(sample))
        {
            stack.samples = stack.samples.saturating_add(1);
            return Ok(())
        }
        if let Some(slot) = self.stacks.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(ProfileStack::from_sample(sample));
            return Ok(())
        }
        self.dropped = self.dropped.saturating_add(1);
        Err(ProfileError::Capacity)
    }

    pub fn ingest_ring<const RING: usize>(&mut self, ring: &ProfileRing<RING>) -> usize {
        let mut count = 0;
        while let Some(sample) = ring.try_pop() {
            let _ = self.ingest(sample);
            count += 1;
        }
        let ring_dropped = ring.dropped();
        self.dropped = self
            .dropped
            .saturating_add(ring_dropped.saturating_sub(self.observed_ring_dropped));
        self.observed_ring_dropped = ring_dropped;
        count
    }

    pub const fn sample_count(&self) -> u64 {
        self.sample_count
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn stacks(&self) -> impl Iterator<Item = ProfileStack> + '_ {
        self.stacks.iter().flatten().copied()
    }

    pub fn encoded_len(&self) -> usize {
        PROFILE_HEADER_BYTES + self.stacks().count() * PROFILE_STACK_RECORD_BYTES
    }

    pub fn encode(&self, destination: &mut [u8]) -> Result<usize, ProfileError> {
        let required = self.encoded_len();
        if destination.len() < required {
            return Err(ProfileError::BufferTooSmall { required });
        }
        destination[..required].fill(0);
        destination[..8].copy_from_slice(PROFILE_MAGIC);
        destination[8..10].copy_from_slice(&PROFILE_VERSION.to_le_bytes());
        destination[10..12].copy_from_slice(&(PROFILE_HEADER_BYTES as u16).to_le_bytes());
        destination[12..44].copy_from_slice(self.metadata.revision.as_bytes());
        destination[44..60].copy_from_slice(&self.metadata.host_id);
        destination[60..68].copy_from_slice(&self.metadata.sample_period_us.to_le_bytes());
        destination[68..76].copy_from_slice(&self.metadata.started_at.to_le_bytes());
        destination[76..84].copy_from_slice(&self.metadata.ended_at.to_le_bytes());
        destination[84..92].copy_from_slice(&self.sample_count.to_le_bytes());
        destination[92..100].copy_from_slice(&self.dropped.to_le_bytes());
        let stacks = self.stacks().count() as u32;
        destination[100..104].copy_from_slice(&stacks.to_le_bytes());
        let mut offset = PROFILE_HEADER_BYTES;
        for stack in self.stacks() {
            destination[offset] = stack.domain as u8;
            destination[offset + 1] = stack.depth;
            destination[offset + 4..offset + 12].copy_from_slice(&stack.samples.to_le_bytes());
            for (index, frame) in stack.frames().enumerate() {
                let frame_offset = offset + 12 + index * 12;
                destination[frame_offset..frame_offset + 8]
                    .copy_from_slice(&frame.symbol.to_le_bytes());
                destination[frame_offset + 8..frame_offset + 12]
                    .copy_from_slice(&frame.offset.to_le_bytes());
            }
            offset += PROFILE_STACK_RECORD_BYTES;
        }
        let checksum = profile_checksum(&destination[..required]);
        destination[PROFILE_CHECKSUM_OFFSET..PROFILE_CHECKSUM_OFFSET + 8]
            .copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }
}

/// Validate an exported archive without allocating or trusting its counts.
pub fn validate_profile_export(source: &[u8]) -> Result<usize, ProfileError> {
    if source.len() < PROFILE_HEADER_BYTES
        || &source[..8] != PROFILE_MAGIC
        || u16::from_le_bytes([source[8], source[9]]) != PROFILE_VERSION
        || u16::from_le_bytes([source[10], source[11]]) as usize != PROFILE_HEADER_BYTES
    {
        return Err(ProfileError::InvalidExport);
    }
    let stack_count = u32::from_le_bytes(source[100..104].try_into().unwrap()) as usize;
    let required = PROFILE_HEADER_BYTES
        .checked_add(stack_count.checked_mul(PROFILE_STACK_RECORD_BYTES).ok_or(ProfileError::InvalidExport)?)
        .ok_or(ProfileError::InvalidExport)?;
    if source.len() < required
        || u64::from_le_bytes(source[PROFILE_CHECKSUM_OFFSET..PROFILE_CHECKSUM_OFFSET + 8].try_into().unwrap())
            != profile_checksum(&source[..required])
    {
        return Err(ProfileError::InvalidExport);
    }
    Ok(required)
}

fn profile_checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for (index, byte) in bytes.iter().enumerate() {
        if (PROFILE_CHECKSUM_OFFSET..PROFILE_CHECKSUM_OFFSET + 8).contains(&index) {
            continue
        }
        hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedProfile {
    pub revision: ContentId,
    pub host_id: [u8; 16],
    pub sample_count: u64,
}

/// Fixed-capacity index used by a daemon to retain one profile per revision
/// and host. The archive bytes can live in SynFS under the same key.
pub struct ProfileRetention<const CAPACITY: usize> {
    entries: [Option<RetainedProfile>; CAPACITY],
}

impl<const CAPACITY: usize> ProfileRetention<CAPACITY> {
    pub const fn new() -> Self {
        Self { entries: [None; CAPACITY] }
    }

    pub fn retain(&mut self, profile: RetainedProfile) -> Result<(), ProfileError> {
        if let Some(slot) = self.entries.iter_mut().flatten().find(|entry| {
            entry.revision == profile.revision && entry.host_id == profile.host_id
        }) {
            *slot = profile;
            return Ok(())
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ProfileError::Capacity)?;
        *slot = Some(profile);
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = RetainedProfile> + '_ {
        self.entries.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for ProfileRetention<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
