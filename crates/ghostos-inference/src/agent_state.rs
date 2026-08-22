use core::str;

use ghostos_ghostfs::{CheckpointInfo, Error as FileError, MAX_PATH_BYTES, RmsMapHandle, SynFs};

use crate::Error;

const STATE_MAGIC: &[u8; 8] = b"SYNAGNT1";
const STATE_FORMAT_VERSION: u16 = 1;
const STATE_HEADER_BYTES: usize = 56;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct AgentId(u64);

impl AgentId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotPolicy {
    pub interval_us: u64,
}

impl SnapshotPolicy {
    pub const fn new(interval_us: u64) -> Option<Self> {
        if interval_us == 0 {
            None
        } else {
            Some(Self { interval_us })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentSnapshot {
    pub agent: AgentId,
    pub sequence: u64,
    pub created_at_us: u64,
    pub state_bytes: usize,
    pub file_version: u32,
    pub checkpoint: CheckpointInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoredAgentState {
    pub agent: AgentId,
    pub sequence: u64,
    pub created_at_us: u64,
    pub state_bytes: usize,
    pub file_version: u32,
}

#[derive(Clone, Copy)]
struct AgentEntry {
    occupied: bool,
    agent: Option<AgentId>,
    policy: SnapshotPolicy,
    sequence: u64,
    last_snapshot_us: u64,
    state_bytes: usize,
    file_version: u32,
    checkpoint: Option<CheckpointInfo>,
}

impl AgentEntry {
    const EMPTY: Self = Self {
        occupied: false,
        agent: None,
        policy: SnapshotPolicy { interval_us: 1 },
        sequence: 0,
        last_snapshot_us: 0,
        state_bytes: 0,
        file_version: 0,
        checkpoint: None,
    };
}

/// Periodic crash-consistent snapshots for long-running agent processes.
///
/// Every image is a normal immutable GhostFS file version. The newest image also
/// pins its whole filesystem generation, so related stack files can be restored
/// from the same CoW root after a process or node restart.
pub struct AgentSnapshotter<const AGENTS: usize, const STATE_BYTES: usize> {
    agents: [AgentEntry; AGENTS],
}

impl<const AGENTS: usize, const STATE_BYTES: usize> AgentSnapshotter<AGENTS, STATE_BYTES> {
    pub const fn new() -> Self {
        Self {
            agents: [AgentEntry::EMPTY; AGENTS],
        }
    }

    pub fn register(&mut self, agent: AgentId, policy: SnapshotPolicy) -> Result<(), Error> {
        if self
            .agents
            .iter()
            .any(|entry| entry.occupied && entry.agent == Some(agent))
        {
            return Err(Error::DuplicateAgent);
        }
        let slot = self
            .agents
            .iter_mut()
            .find(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;
        *slot = AgentEntry {
            occupied: true,
            agent: Some(agent),
            policy,
            sequence: 0,
            last_snapshot_us: 0,
            state_bytes: 0,
            file_version: 0,
            checkpoint: None,
        };
        Ok(())
    }

    pub fn unregister<const MAX_BLOCKS: usize>(
        &mut self,
        agent: AgentId,
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<(), Error> {
        let slot = self.agent_slot(agent)?;
        if let Some(checkpoint) = self.agents[slot].checkpoint {
            filesystem.release_checkpoint(checkpoint.id)?
        }
        self.agents[slot].occupied = false;
        Ok(())
    }

    pub fn snapshot_if_due<const MAX_BLOCKS: usize>(
        &mut self,
        agent: AgentId,
        sequence: u64,
        now_us: u64,
        state: &[u8],
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<Option<AgentSnapshot>, Error> {
        let slot = self.agent_slot(agent)?;
        let entry = self.agents[slot];
        if sequence <= entry.sequence {
            return Err(Error::StaleSequence);
        }
        if entry.checkpoint.is_some()
            && now_us.saturating_sub(entry.last_snapshot_us) < entry.policy.interval_us
        {
            return Ok(None);
        }
        self.snapshot_slot(slot, sequence, now_us, state, filesystem)
            .map(Some)
    }

    pub fn force_snapshot<const MAX_BLOCKS: usize>(
        &mut self,
        agent: AgentId,
        sequence: u64,
        now_us: u64,
        state: &[u8],
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<AgentSnapshot, Error> {
        let slot = self.agent_slot(agent)?;
        if sequence <= self.agents[slot].sequence {
            return Err(Error::StaleSequence);
        }
        self.snapshot_slot(slot, sequence, now_us, state, filesystem)
    }

    pub fn latest(&self, agent: AgentId) -> Result<AgentSnapshot, Error> {
        let entry = self.agents[self.agent_slot(agent)?];
        let checkpoint = entry.checkpoint.ok_or(Error::AgentNotFound)?;
        Ok(AgentSnapshot {
            agent,
            sequence: entry.sequence,
            created_at_us: entry.last_snapshot_us,
            state_bytes: entry.state_bytes,
            file_version: entry.file_version,
            checkpoint,
        })
    }

    pub fn restore_latest<const MAX_BLOCKS: usize>(
        &self,
        agent: AgentId,
        capability: RmsMapHandle,
        filesystem: &SynFs<MAX_BLOCKS>,
        destination: &mut [u8],
    ) -> Result<AgentSnapshot, Error> {
        let expected = self.latest(agent)?;
        let snapshot = filesystem.checkpoint_snapshot(expected.checkpoint.id, capability)?;
        let mut path_buffer = [0; MAX_PATH_BYTES];
        let path = agent_path(agent, &mut path_buffer)?;
        let file = snapshot.lookup(path)?;
        if file.version != expected.file_version
            || file.size as usize > STATE_BYTES
            || (file.size as usize) < STATE_HEADER_BYTES
        {
            return Err(Error::CorruptState);
        }
        let encoded_length = file.size as usize;
        let mut encoded = [0; STATE_BYTES];
        let mut copied = 0;
        let mut invalid = false;
        snapshot.visit_file_pages(path, |page| {
            let start = page.file_offset as usize;
            let Some(end) = start.checked_add(page.bytes.len()) else {
                invalid = true;
                return;
            };
            if end > encoded_length || start != copied {
                invalid = true;
                return;
            }
            encoded[start..end].copy_from_slice(page.bytes);
            copied = end;
        })?;
        if invalid || copied != encoded_length {
            return Err(Error::CorruptState);
        }
        let decoded = decode_state(agent, &encoded[..encoded_length], destination)?;
        if decoded.sequence != expected.sequence || decoded.created_at_us != expected.created_at_us
        {
            return Err(Error::CorruptState);
        }
        Ok(AgentSnapshot {
            state_bytes: decoded.state_bytes,
            ..expected
        })
    }

    pub fn restore_version<const MAX_BLOCKS: usize>(
        &self,
        agent: AgentId,
        version: u32,
        filesystem: &SynFs<MAX_BLOCKS>,
        destination: &mut [u8],
    ) -> Result<RestoredAgentState, Error> {
        let mut path_buffer = [0; MAX_PATH_BYTES];
        let path = agent_path(agent, &mut path_buffer)?;
        let mut encoded = [0; STATE_BYTES];
        let read = filesystem.read_version(path, version, &mut encoded)?;
        let decoded = decode_state(agent, &encoded[..read.bytes_read], destination)?;
        Ok(RestoredAgentState {
            agent,
            sequence: decoded.sequence,
            created_at_us: decoded.created_at_us,
            state_bytes: decoded.state_bytes,
            file_version: version,
        })
    }

    fn snapshot_slot<const MAX_BLOCKS: usize>(
        &mut self,
        slot: usize,
        sequence: u64,
        now_us: u64,
        state: &[u8],
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<AgentSnapshot, Error> {
        let agent = self.agents[slot].agent.ok_or(Error::AgentNotFound)?;
        let required = STATE_HEADER_BYTES
            .checked_add(state.len())
            .ok_or(Error::Capacity)?;
        if required > STATE_BYTES {
            return Err(Error::BufferTooSmall { required });
        }
        let mut encoded = [0; STATE_BYTES];
        encode_state(agent, sequence, now_us, state, &mut encoded[..required]);
        let mut path_buffer = [0; MAX_PATH_BYTES];
        let path = agent_path(agent, &mut path_buffer)?;
        let (file, transaction_commit) = {
            let mut transaction = filesystem.transaction();
            let file = transaction.write(path, &encoded[..required])?;
            let transaction_commit = transaction.commit()?;
            (file, transaction_commit)
        };
        let old_checkpoint = self.agents[slot].checkpoint;
        let checkpoint = match filesystem.create_checkpoint() {
            Ok(checkpoint) => checkpoint,
            Err(FileError::TooManyCheckpoints) => {
                let old = old_checkpoint.ok_or(Error::Capacity)?;
                filesystem.release_checkpoint(old.id)?;
                filesystem.create_checkpoint()?
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(old) = old_checkpoint {
            if old.id != checkpoint.id && filesystem.checkpoint_info(old.id).is_ok() {
                filesystem.release_checkpoint(old.id)?
            }
        }
        if checkpoint.generation != transaction_commit.generation {
            return Err(Error::CorruptState);
        }
        self.agents[slot].sequence = sequence;
        self.agents[slot].last_snapshot_us = now_us;
        self.agents[slot].state_bytes = state.len();
        self.agents[slot].file_version = file.version;
        self.agents[slot].checkpoint = Some(checkpoint);
        Ok(AgentSnapshot {
            agent,
            sequence,
            created_at_us: now_us,
            state_bytes: state.len(),
            file_version: file.version,
            checkpoint,
        })
    }

    fn agent_slot(&self, agent: AgentId) -> Result<usize, Error> {
        self.agents
            .iter()
            .position(|entry| entry.occupied && entry.agent == Some(agent))
            .ok_or(Error::AgentNotFound)
    }
}

impl<const AGENTS: usize, const STATE_BYTES: usize> Default
    for AgentSnapshotter<AGENTS, STATE_BYTES>
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct DecodedState {
    sequence: u64,
    created_at_us: u64,
    state_bytes: usize,
}

fn encode_state(
    agent: AgentId,
    sequence: u64,
    created_at_us: u64,
    state: &[u8],
    destination: &mut [u8],
) {
    destination[..8].copy_from_slice(STATE_MAGIC);
    destination[8..10].copy_from_slice(&STATE_FORMAT_VERSION.to_le_bytes());
    destination[12..16].copy_from_slice(&(STATE_HEADER_BYTES as u32).to_le_bytes());
    destination[16..24].copy_from_slice(&agent.raw().to_le_bytes());
    destination[24..32].copy_from_slice(&sequence.to_le_bytes());
    destination[32..40].copy_from_slice(&created_at_us.to_le_bytes());
    destination[40..48].copy_from_slice(&(state.len() as u64).to_le_bytes());
    destination[48..56].copy_from_slice(&state_checksum(state).to_le_bytes());
    destination[STATE_HEADER_BYTES..].copy_from_slice(state);
}

fn decode_state(
    expected_agent: AgentId,
    encoded: &[u8],
    destination: &mut [u8],
) -> Result<DecodedState, Error> {
    if encoded.len() < STATE_HEADER_BYTES
        || &encoded[..8] != STATE_MAGIC
        || read_u16(encoded, 8) != STATE_FORMAT_VERSION
        || read_u32(encoded, 12) as usize != STATE_HEADER_BYTES
        || read_u64(encoded, 16) != expected_agent.raw()
    {
        return Err(Error::CorruptState);
    }
    let state_bytes = usize::try_from(read_u64(encoded, 40)).map_err(|_| Error::CorruptState)?;
    let required = STATE_HEADER_BYTES
        .checked_add(state_bytes)
        .ok_or(Error::CorruptState)?;
    if encoded.len() != required
        || state_checksum(&encoded[STATE_HEADER_BYTES..]) != read_u64(encoded, 48)
    {
        return Err(Error::CorruptState);
    }
    if destination.len() < state_bytes {
        return Err(Error::BufferTooSmall {
            required: state_bytes,
        });
    }
    destination[..state_bytes].copy_from_slice(&encoded[STATE_HEADER_BYTES..]);
    Ok(DecodedState {
        sequence: read_u64(encoded, 24),
        created_at_us: read_u64(encoded, 32),
        state_bytes,
    })
}

fn agent_path<'a>(
    agent: AgentId,
    destination: &'a mut [u8; MAX_PATH_BYTES],
) -> Result<&'a str, Error> {
    const PREFIX: &[u8] = b"SYS$AGENTS/";
    const SUFFIX: &[u8] = b".STATE";
    let required = PREFIX.len() + 16 + SUFFIX.len();
    destination[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut value = agent.raw();
    for index in (PREFIX.len()..PREFIX.len() + 16).rev() {
        let digit = (value & 0xf) as u8;
        destination[index] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        value >>= 4;
    }
    destination[PREFIX.len() + 16..required].copy_from_slice(SUFFIX);
    str::from_utf8(&destination[..required]).map_err(|_| Error::CorruptState)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn state_checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}
