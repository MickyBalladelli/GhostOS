//! VM checkpoints, portable serialization, page diffs, and snapshot chains.

use crate::cpu::{CpuMode, CpuState, DescriptorTableRegister, PrivilegeLevel, SegmentRegister};
use crate::devices::{InterruptControllerState, LocalApicState};
use crate::firmware::bios::BiosState;
use crate::memory::{MmuState, PAGE_SIZE};
use crate::Vm;
use bitflags::bitflags;
use std::fs;
use std::fmt;
use std::path::Path;

const MAGIC: &[u8; 8] = b"SYNOVM01";
const AUTH_MAGIC: &[u8; 8] = b"SYNOSIG1";
const AUTH_MAGIC_GHOSTOS: &[u8; 8] = b"GHOSTSG1";
const AUTH_ALGORITHM_HMAC_SHA256: u8 = 1;
const AUTH_HEADER_BYTES: usize = 8 + 4 + 1 + 3 + 16 + 8;
const AUTH_TAG_BYTES: usize = 32;
pub const SNAPSHOT_AUTH_FORMAT_VERSION: u32 = 1;
pub const SNAPSHOT_AUTH_KEY_BYTES: usize = 32;
pub const SNAPSHOT_AUTH_TAG_BYTES: usize = AUTH_TAG_BYTES;
pub const SNAPSHOT_FORMAT_VERSION: u32 = 2;
pub const SNAPSHOT_MIN_FORMAT_VERSION: u32 = 1;
pub const SNAPSHOT_API_VERSION: ghostos_api_compat::ApiVersion = ghostos_api_compat::SNAPSHOT_API.current;
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_SNAPSHOT_MEMORY_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_ITEMS: usize = 16 * 1024 * 1024;

fn auth_magic_accepted(magic: &[u8]) -> bool {
    magic == AUTH_MAGIC || magic == AUTH_MAGIC_GHOSTOS
}

pub type SnapshotId = u64;

/// Stable digest used as a replay-guard identity for an authenticated
/// checkpoint payload.
pub fn snapshot_digest(bytes: &[u8]) -> [u8; 32] {
    sha256(bytes)
}

/// Shared secret used to authenticate snapshot files and migration frames.
/// The key is never serialized; only its non-secret identifier is carried on
/// the wire so both peers can select the same configured key.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SnapshotAuthKey([u8; SNAPSHOT_AUTH_KEY_BYTES]);

impl SnapshotAuthKey {
    pub const fn new(bytes: [u8; SNAPSHOT_AUTH_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() != SNAPSHOT_AUTH_KEY_BYTES {
            return Err(SnapshotError::InvalidAuthenticationKey)
        }
        let mut key = [0u8; SNAPSHOT_AUTH_KEY_BYTES];
        key.copy_from_slice(bytes);
        Ok(Self::new(key))
    }

    pub fn from_hex(value: &str) -> Result<Self, SnapshotError> {
        let value = value.trim();
        if value.len() != SNAPSHOT_AUTH_KEY_BYTES * 2 {
            return Err(SnapshotError::InvalidAuthenticationKey)
        }
        let mut key = [0u8; SNAPSHOT_AUTH_KEY_BYTES];
        for (index, byte) in key.iter_mut().enumerate() {
            let start = index * 2;
            let high = hex_value(value.as_bytes()[start])
                .ok_or(SnapshotError::InvalidAuthenticationKey)?;
            let low = hex_value(value.as_bytes()[start + 1])
                .ok_or(SnapshotError::InvalidAuthenticationKey)?;
            *byte = (high << 4) | low;
        }
        Ok(Self::new(key))
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let bytes = fs::read(path)?;
        if bytes.len() == SNAPSHOT_AUTH_KEY_BYTES {
            return Self::from_bytes(&bytes)
        }
        Self::from_hex(std::str::from_utf8(&bytes).map_err(|_| SnapshotError::InvalidAuthenticationKey)?)
    }

    pub fn key_id(self) -> [u8; 16] {
        let digest = sha256(&self.0);
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        id
    }

    pub fn authenticate_parts(self, parts: &[&[u8]]) -> [u8; AUTH_TAG_BYTES] {
        hmac_sha256(&self.0, parts)
    }

    pub fn verify_parts(
        self,
        parts: &[&[u8]],
        tag: &[u8; AUTH_TAG_BYTES],
    ) -> Result<(), SnapshotError> {
        let expected = self.authenticate_parts(parts);
        self.verify_tag(&expected, tag)
    }

    pub fn verify_tag(
        self,
        expected: &[u8; AUTH_TAG_BYTES],
        tag: &[u8; AUTH_TAG_BYTES],
    ) -> Result<(), SnapshotError> {
        if constant_time_equal(expected, tag) {
            Ok(())
        } else {
            Err(SnapshotError::AuthenticationFailed)
        }
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for SnapshotAuthKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotAuthKey")
            .field("key_id", &self.key_id())
            .finish()
    }
}

bitflags! {
    /// State components understood by the snapshot wire format.
    ///
    /// All currently defined components are required by both supported
    /// formats. A future optional component must get its own bit and a
    /// decoder path before it is advertised during negotiation.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SnapshotFeatures: u64 {
        const CPU_STATE = 1 << 0;
        const MMU_STATE = 1 << 1;
        const INTERRUPT_CONTROLLER = 1 << 2;
        const APIC_STATE = 1 << 3;
        const BIOS_STATE = 1 << 4;
        const ALL = Self::CPU_STATE.bits()
            | Self::MMU_STATE.bits()
            | Self::INTERRUPT_CONTROLLER.bits()
            | Self::APIC_STATE.bits()
            | Self::BIOS_STATE.bits();
    }
}

/// Version and feature range advertised by a snapshot or migration peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotSchema {
    pub min_version: u32,
    pub max_version: u32,
    pub features: SnapshotFeatures,
}

impl SnapshotSchema {
    pub const fn local() -> Self {
        Self {
            min_version: SNAPSHOT_MIN_FORMAT_VERSION,
            max_version: SNAPSHOT_FORMAT_VERSION,
            features: SnapshotFeatures::ALL,
        }
    }

    pub const fn for_version(version: u32) -> Self {
        Self {
            min_version: version,
            max_version: version,
            features: SnapshotFeatures::ALL,
        }
    }

    /// Validate a schema advertisement before using it for negotiation.
    pub fn validate(self) -> Result<(), SnapshotError> {
        if self.min_version > self.max_version {
            return Err(SnapshotError::InvalidSchema {
                min_version: self.min_version,
                max_version: self.max_version,
            })
        }
        let unsupported = self.features.bits() & !SnapshotFeatures::ALL.bits();
        if unsupported != 0 {
            return Err(SnapshotError::UnsupportedFeatures { unsupported })
        }
        Ok(())
    }

    /// Negotiate with the local implementation's schema.
    pub fn negotiate(peer: Self) -> Result<Self, SnapshotError> {
        Self::negotiate_with(Self::local(), peer)
    }

    /// Negotiate a single wire version and the feature set it can carry.
    pub fn negotiate_with(local: Self, peer: Self) -> Result<Self, SnapshotError> {
        local.validate()?;
        peer.validate()?;
        let min_version = self_min(local.min_version, peer.min_version);
        let max_version = self_max(local.max_version, peer.max_version);
        if min_version > max_version {
            return Err(SnapshotError::SchemaMismatch {
                local_min: local.min_version,
                local_max: local.max_version,
                peer_min: peer.min_version,
                peer_max: peer.max_version,
            })
        }
        let version = max_version;
        let features = local.features & peer.features;
        let missing = (required_features(version) - features).bits();
        if missing != 0 {
            return Err(SnapshotError::MissingFeatures {
                missing,
            })
        }
        Ok(Self {
            min_version: version,
            max_version: version,
            features,
        })
    }

    pub fn accepts(&self, version: u32, features: SnapshotFeatures) -> bool {
        version >= self.min_version
            && version <= self.max_version
            && (features - SnapshotFeatures::ALL).is_empty()
            && features.contains(required_features(version))
            && (features - self.features).is_empty()
    }
}

const fn required_features(_version: u32) -> SnapshotFeatures {
    SnapshotFeatures::ALL
}

const fn self_min(left: u32, right: u32) -> u32 {
    if left > right { left } else { right }
}

const fn self_max(left: u32, right: u32) -> u32 {
    if left < right { left } else { right }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid snapshot format")]
    InvalidFormat,
    #[error("unsupported snapshot version {0}")]
    VersionMismatch(u32),
    #[error("snapshot schema ranges do not overlap: local {local_min}..={local_max}, peer {peer_min}..={peer_max}")]
    SchemaMismatch {
        local_min: u32,
        local_max: u32,
        peer_min: u32,
        peer_max: u32,
    },
    #[error("invalid snapshot schema range {min_version}..={max_version}")]
    InvalidSchema { min_version: u32, max_version: u32 },
    #[error("snapshot requires unsupported feature bits 0x{unsupported:016x}")]
    UnsupportedFeatures { unsupported: u64 },
    #[error("snapshot is missing required feature bits 0x{missing:016x}")]
    MissingFeatures { missing: u64 },
    #[error("snapshot exceeds the {max} byte decode limit")]
    SizeLimit { max: u64 },
    #[error("snapshot belongs to a VM with {expected} bytes of RAM, not {actual}")]
    IncompatibleMemory { expected: usize, actual: usize },
    #[error("snapshot state is invalid")]
    InvalidState,
    #[error("snapshot authentication key must be exactly 32 bytes")]
    InvalidAuthenticationKey,
    #[error("snapshot is not authenticated")]
    AuthenticationRequired,
    #[error("snapshot authentication failed")]
    AuthenticationFailed,
    #[error("unsupported snapshot authentication version {0}")]
    UnsupportedAuthenticationVersion(u32),
    #[error("snapshot checksum does not match its base")]
    ChecksumMismatch,
    #[error("snapshot {0} does not exist in the chain")]
    MissingSnapshot(SnapshotId),
}

/// Complete guest checkpoint. Host-side translation caches and network
/// backends are deliberately excluded; they are rebuilt or remain attached
/// to the VM topology after restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmSnapshot {
    pub format_version: u32,
    pub feature_flags: SnapshotFeatures,
    pub memory_size: usize,
    pub cpu: CpuState,
    pub mmu: MmuState,
    pub interrupt_controller: InterruptControllerState,
    pub apic: LocalApicState,
    pub bios_state: BiosState,
    pub bios_ivt: [u16; 256],
    pub bios_bda: [u8; 256],
    pub bios_ega: [u8; 32 * 4],
    pub bios_reset_vector: u64,
}

const SNAPSHOT_RESTORED_STATE: &[&str] = &[
    "CPU registers and execution state",
    "guest RAM and MMU/page-table state",
    "interrupt controller state",
    "local APIC state",
    "BIOS lifecycle state and BIOS tables",
];

const SNAPSHOT_REBUILT_STATE: &[&str] = &[
    "port and MMIO device topology",
    "PCI topology and BAR defaults",
    "disk and network backend handles",
    "host clock and hardware-acceleration handles",
    "translation cache",
];

const SNAPSHOT_EXCLUDED_STATE: &[&str] = &[
    "serial, PS/2, and virtio-console input/output queues",
    "device queues and in-flight DMA requests",
    "PIT and HPET host-time progress",
    "display host state",
    "guest-agent, hotplug, and power-notification queues",
    "virtio RNG host entropy state",
    "external disk contents and host cache/lock state",
    "network backend queues and link state",
    "UEFI runtime/application host pointers",
];

/// Explains what a snapshot restore did and which host-owned state must be
/// recreated or remains outside the checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRestoreReport {
    restored_state: &'static [&'static str],
    rebuild_required_state: &'static [&'static str],
    excluded_state: &'static [&'static str],
}

impl SnapshotRestoreReport {
    fn new() -> Self {
        Self {
            restored_state: SNAPSHOT_RESTORED_STATE,
            rebuild_required_state: SNAPSHOT_REBUILT_STATE,
            excluded_state: SNAPSHOT_EXCLUDED_STATE,
        }
    }

    pub fn restored_state(&self) -> &'static [&'static str] {
        self.restored_state
    }

    pub fn rebuild_required_state(&self) -> &'static [&'static str] {
        self.rebuild_required_state
    }

    pub fn excluded_state(&self) -> &'static [&'static str] {
        self.excluded_state
    }

    pub fn has_excluded_state(&self) -> bool {
        !self.excluded_state.is_empty()
    }
}

impl VmSnapshot {
    pub fn capture(vm: &Vm) -> Self {
        Self {
            format_version: SNAPSHOT_FORMAT_VERSION,
            feature_flags: SnapshotFeatures::ALL,
            memory_size: vm.mmu.ram_size(),
            cpu: vm.cpu.state,
            mmu: vm.mmu.snapshot_state(),
            interrupt_controller: vm.interrupt_controller.snapshot_state(),
            apic: vm.apic.borrow().snapshot_state(),
            bios_state: vm.bios.context.state,
            bios_ivt: vm.bios.context.ivt,
            bios_bda: vm.bios.context.bda,
            bios_ega: vm.bios.context.ega,
            bios_reset_vector: vm.bios.reset_vector,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.encode_bytes()
    }

    /// Encode after checking the in-memory state and wire limits.
    pub fn try_to_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        self.validate()?;
        let bytes = self.encode_bytes();
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        Ok(bytes)
    }

    /// Encode the snapshot inside an authenticated envelope.
    pub fn to_authenticated_bytes(
        &self,
        key: SnapshotAuthKey,
    ) -> Result<Vec<u8>, SnapshotError> {
        let payload = self.try_to_bytes()?;
        let payload_length = u64::try_from(payload.len()).map_err(|_| SnapshotError::SizeLimit {
            max: MAX_SNAPSHOT_BYTES,
        })?;
        let total_length = AUTH_HEADER_BYTES
            .checked_add(payload.len())
            .and_then(|length| length.checked_add(AUTH_TAG_BYTES))
            .ok_or(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })?;
        if u64::try_from(total_length).unwrap_or(u64::MAX) > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }

        let mut envelope = Vec::with_capacity(total_length);
        envelope.extend_from_slice(AUTH_MAGIC);
        envelope.extend_from_slice(&SNAPSHOT_AUTH_FORMAT_VERSION.to_le_bytes());
        envelope.push(AUTH_ALGORITHM_HMAC_SHA256);
        envelope.extend_from_slice(&[0u8; 3]);
        envelope.extend_from_slice(&key.key_id());
        envelope.extend_from_slice(&payload_length.to_le_bytes());
        envelope.extend_from_slice(&payload);
        let tag = key.authenticate_parts(&[&envelope]);
        envelope.extend_from_slice(&tag);
        Ok(envelope)
    }

    /// Verify an authenticated envelope before decoding its snapshot payload.
    pub fn from_authenticated_bytes(
        bytes: &[u8],
        key: SnapshotAuthKey,
    ) -> Result<Self, SnapshotError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        if bytes.len() < AUTH_HEADER_BYTES + AUTH_TAG_BYTES
            || !auth_magic_accepted(bytes.get(..AUTH_MAGIC.len()).unwrap_or(&[]))
        {
            return Err(SnapshotError::AuthenticationRequired)
        }
        let version = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| SnapshotError::InvalidFormat)?,
        );
        if version != SNAPSHOT_AUTH_FORMAT_VERSION {
            return Err(SnapshotError::UnsupportedAuthenticationVersion(version))
        }
        if bytes[12] != AUTH_ALGORITHM_HMAC_SHA256 || bytes[13..16] != [0u8; 3] {
            return Err(SnapshotError::InvalidFormat)
        }
        let mut key_id = [0u8; 16];
        key_id.copy_from_slice(&bytes[16..32]);
        let payload_length = u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .map_err(|_| SnapshotError::InvalidFormat)?,
        );
        let payload_length = usize::try_from(payload_length)
            .map_err(|_| SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })?;
        let payload_end = AUTH_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or(SnapshotError::InvalidFormat)?;
        let tag_start = payload_end;
        let expected_length = tag_start
            .checked_add(AUTH_TAG_BYTES)
            .ok_or(SnapshotError::InvalidFormat)?;
        if expected_length != bytes.len() {
            return Err(SnapshotError::InvalidFormat)
        }
        if !constant_time_equal_short(&key_id, &key.key_id()) {
            return Err(SnapshotError::AuthenticationFailed)
        }
        let tag: &[u8; AUTH_TAG_BYTES] = bytes[tag_start..]
            .try_into()
            .map_err(|_| SnapshotError::InvalidFormat)?;
        key.verify_parts(&[&bytes[..tag_start]], tag)?;
        Self::from_bytes(&bytes[AUTH_HEADER_BYTES..payload_end])
    }

    fn encode_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.bytes(MAGIC);
        writer.u32(self.format_version);
        if self.format_version >= 2 {
            writer.u64(self.feature_flags.bits());
        }
        writer.u64(self.memory_size as u64);
        encode_cpu(&mut writer, &self.cpu);
        encode_mmu(&mut writer, &self.mmu);
        encode_interrupt_controller(&mut writer, &self.interrupt_controller);
        encode_apic(&mut writer, &self.apic);
        writer.u8(encode_bios_state(self.bios_state));
        for value in self.bios_ivt {
            writer.u16(value);
        }
        writer.bytes(&self.bios_bda);
        writer.bytes(&self.bios_ega);
        writer.u64(self.bios_reset_vector);
        writer.finish()
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.to_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        let mut reader = Reader::new(bytes);
        if reader.bytes_exact(8)? != MAGIC {
            return Err(SnapshotError::InvalidFormat)
        }
        let format_version = reader.u32()?;
        let feature_flags = match format_version {
            1 => SnapshotFeatures::ALL,
            2 => {
                let raw = reader.u64()?;
                let unsupported = raw & !SnapshotFeatures::ALL.bits();
                if unsupported != 0 {
                    return Err(SnapshotError::UnsupportedFeatures { unsupported })
                }
                let flags = SnapshotFeatures::from_bits_retain(raw);
                if !flags.contains(SnapshotFeatures::ALL) {
                    return Err(SnapshotError::MissingFeatures {
                        missing: (SnapshotFeatures::ALL - flags).bits(),
                    })
                }
                flags
            }
            _ => {
                return Err(SnapshotError::VersionMismatch(format_version))
            }
        };
        if !(SNAPSHOT_MIN_FORMAT_VERSION..=SNAPSHOT_FORMAT_VERSION).contains(&format_version) {
            return Err(SnapshotError::VersionMismatch(format_version))
        }
        let memory_size = reader.u64()?;
        if memory_size > MAX_SNAPSHOT_MEMORY_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        let memory_size = usize::try_from(memory_size).map_err(|_| SnapshotError::InvalidFormat)?;
        let cpu = decode_cpu(&mut reader)?;
        let mmu = decode_mmu(&mut reader, memory_size)?;
        if mmu.ram.len() != memory_size {
            return Err(SnapshotError::InvalidFormat)
        }
        let interrupt_controller = decode_interrupt_controller(&mut reader)?;
        let apic = decode_apic(&mut reader)?;
        let bios_state = decode_bios_state(reader.u8()?)?;
        let mut bios_ivt = [0u16; 256];
        for value in &mut bios_ivt {
            *value = reader.u16()?
        }
        let bios_bda = reader.array::<256>()?;
        let bios_ega = reader.array::<128>()?;
        let bios_reset_vector = reader.u64()?;
        if !reader.is_empty() {
            return Err(SnapshotError::InvalidFormat)
        }
        let snapshot = Self {
            format_version,
            feature_flags,
            memory_size,
            cpu,
            mmu,
            interrupt_controller,
            apic,
            bios_state,
            bios_ivt,
            bios_bda,
            bios_ega,
            bios_reset_vector,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, SnapshotError> {
        Self::from_bytes(bytes)
    }

    /// Save the raw payload for offline format conversion only. It has no
    /// authenticity and must not be used as a trusted checkpoint.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SnapshotError> {
        fs::write(path, self.try_to_bytes()?).map_err(SnapshotError::Io)
    }

    pub fn save_authenticated(
        &self,
        path: impl AsRef<Path>,
        key: SnapshotAuthKey,
    ) -> Result<(), SnapshotError> {
        fs::write(path, self.to_authenticated_bytes(key)?).map_err(SnapshotError::Io)
    }

    /// Load an untrusted raw payload for offline conversion only.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        Self::from_bytes(&fs::read(path)?)
    }

    pub fn load_authenticated(
        path: impl AsRef<Path>,
        key: SnapshotAuthKey,
    ) -> Result<Self, SnapshotError> {
        Self::from_authenticated_bytes(&fs::read(path)?, key)
    }

    /// Return a local chain identity. This is not an authenticity check.
    pub fn checksum(&self) -> u64 {
        checksum(&self.to_bytes())
    }

    pub fn schema(&self) -> SnapshotSchema {
        SnapshotSchema {
            min_version: self.format_version,
            max_version: self.format_version,
            features: self.feature_flags,
        }
    }

    /// Convert a checkpoint to a negotiated, single-version wire schema.
    ///
    /// Version 1 uses implicit `ALL` feature flags. Version 2 writes the
    /// explicit flags word. The state payload is identical today, so this is
    /// a metadata-only conversion with full validation on both sides.
    pub fn convert_to_schema(&self, schema: SnapshotSchema) -> Result<Self, SnapshotError> {
        self.validate()?;
        schema.validate()?;
        if schema.min_version != schema.max_version {
            return Err(SnapshotError::InvalidSchema {
                min_version: schema.min_version,
                max_version: schema.max_version,
            })
        }
        if !SnapshotSchema::local().accepts(schema.min_version, schema.features) {
            return Err(SnapshotError::SchemaMismatch {
                local_min: SNAPSHOT_MIN_FORMAT_VERSION,
                local_max: SNAPSHOT_FORMAT_VERSION,
                peer_min: schema.min_version,
                peer_max: schema.max_version,
            })
        }
        if !schema.accepts(schema.min_version, self.feature_flags) {
            return Err(SnapshotError::MissingFeatures {
                missing: (required_features(schema.min_version) - self.feature_flags).bits(),
            })
        }
        let mut converted = self.clone();
        converted.format_version = schema.min_version;
        converted.feature_flags = if schema.min_version == 1 {
            SnapshotFeatures::ALL
        } else {
            schema.features
        };
        converted.validate()?;
        Ok(converted)
    }

    /// Validate all decoded state before restore or migration.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if !(SNAPSHOT_MIN_FORMAT_VERSION..=SNAPSHOT_FORMAT_VERSION).contains(&self.format_version) {
            return Err(SnapshotError::VersionMismatch(self.format_version))
        }
        let unsupported = self.feature_flags.bits() & !SnapshotFeatures::ALL.bits();
        if unsupported != 0 {
            return Err(SnapshotError::UnsupportedFeatures { unsupported })
        }
        let missing = (required_features(self.format_version) - self.feature_flags).bits();
        if missing != 0 {
            return Err(SnapshotError::MissingFeatures { missing })
        }
        let memory_size = u64::try_from(self.memory_size).map_err(|_| SnapshotError::InvalidFormat)?;
        if memory_size > MAX_SNAPSHOT_MEMORY_BYTES {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_MEMORY_BYTES,
            })
        }
        if self.mmu.ram.len() != self.memory_size {
            return Err(SnapshotError::InvalidFormat)
        }
        if self.mmu.free_frames.len() > MAX_ITEMS
            || self.mmu.identity.len() > MAX_ITEMS
            || self.mmu.cow_pages.len() > MAX_ITEMS
            || self.mmu.mapped_frames.len() > MAX_ITEMS
            || self.mmu.mapped_pages.len() > MAX_ITEMS
            || self.mmu.ballooned_frames.len() > MAX_ITEMS
        {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        if self.interrupt_controller.irq_routing.len() > MAX_ITEMS {
            return Err(SnapshotError::SizeLimit {
                max: MAX_SNAPSHOT_BYTES,
            })
        }
        Ok(())
    }

    /// Build a page-level delta from `self` to `target`.
    pub fn diff(&self, target: &Self) -> Result<SnapshotDiff, SnapshotError> {
        self.validate()?;
        target.validate()?;
        if self.format_version != target.format_version
            || self.feature_flags != target.feature_flags
        {
            return Err(SnapshotError::SchemaMismatch {
                local_min: self.format_version,
                local_max: self.format_version,
                peer_min: target.format_version,
                peer_max: target.format_version,
            })
        }
        if self.memory_size != target.memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: self.memory_size,
                actual: target.memory_size,
            })
        }
        let mut changed_pages = Vec::new();
        for (page, (before, after)) in self
            .mmu
            .ram
            .chunks(PAGE_SIZE)
            .zip(target.mmu.ram.chunks(PAGE_SIZE))
            .enumerate()
        {
            if before != after {
                changed_pages.push(SnapshotPage {
                    page: page as u64,
                    data: after.to_vec(),
                })
            }
        }
        let mut target_mmu = target.mmu.clone();
        target_mmu.ram.clear();
        Ok(SnapshotDiff {
            base_checksum: self.checksum(),
            target_memory_size: target.memory_size,
            target_feature_flags: target.feature_flags,
            target_cpu: target.cpu,
            target_mmu,
            target_interrupt_controller: target.interrupt_controller.clone(),
            target_apic: target.apic.clone(),
            target_bios_state: target.bios_state,
            target_bios_ivt: target.bios_ivt,
            target_bios_bda: target.bios_bda,
            target_bios_ega: target.bios_ega,
            target_bios_reset_vector: target.bios_reset_vector,
            changed_pages,
        })
    }
}

impl VmSnapshot {
    /// Restore serialized guest state and return the host-state boundary.
    pub fn restore_into_with_report(
        &self,
        vm: &mut Vm,
    ) -> Result<SnapshotRestoreReport, SnapshotError> {
        self.validate()?;
        if vm.mmu.ram_size() != self.memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: vm.mmu.ram_size(),
                actual: self.memory_size,
            })
        }
        vm.mmu
            .restore_state(&self.mmu)
            .map_err(|_| SnapshotError::InvalidState)?;
        vm.cpu.state = self.cpu;
        vm.interrupt_controller.restore_state(&self.interrupt_controller);
        vm.apic.borrow_mut().restore_state(&self.apic);
        vm.bios.context.state = self.bios_state;
        vm.bios.context.ivt = self.bios_ivt;
        vm.bios.context.bda = self.bios_bda;
        vm.bios.context.ega = self.bios_ega;
        vm.bios.reset_vector = self.bios_reset_vector;
        vm.initialized = self.bios_state != BiosState::Reset;
        vm.execution.clear_cache();
        Ok(SnapshotRestoreReport::new())
    }

    pub fn restore_into(&self, vm: &mut Vm) -> Result<(), SnapshotError> {
        self.restore_into_with_report(vm).map(|_| ())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPage {
    pub page: u64,
    pub data: Vec<u8>,
}

/// A diff stores changed RAM pages and complete non-RAM state. This keeps
/// chains small when a guest mostly reuses its memory while preserving exact
/// restore semantics for CPU, paging, firmware, and interrupt state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub base_checksum: u64,
    pub target_memory_size: usize,
    pub target_feature_flags: SnapshotFeatures,
    pub target_cpu: CpuState,
    pub target_mmu: MmuState,
    pub target_interrupt_controller: InterruptControllerState,
    pub target_apic: LocalApicState,
    pub target_bios_state: BiosState,
    pub target_bios_ivt: [u16; 256],
    pub target_bios_bda: [u8; 256],
    pub target_bios_ega: [u8; 128],
    pub target_bios_reset_vector: u64,
    pub changed_pages: Vec<SnapshotPage>,
}

impl SnapshotDiff {
    pub fn apply_to(&self, base: &VmSnapshot) -> Result<VmSnapshot, SnapshotError> {
        base.validate()?;
        if base.checksum() != self.base_checksum {
            return Err(SnapshotError::ChecksumMismatch)
        }
        if base.memory_size != self.target_memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: base.memory_size,
                actual: self.target_memory_size,
            })
        }
        let mut mmu = self.target_mmu.clone();
        mmu.ram = base.mmu.ram.clone();
        for page in &self.changed_pages {
            let start = usize::try_from(
                page
                    .page
                    .checked_mul(PAGE_SIZE as u64)
                    .ok_or(SnapshotError::InvalidState)?,
            )
            .map_err(|_| SnapshotError::InvalidState)?;
            let end = start
                .checked_add(page.data.len())
                .ok_or(SnapshotError::InvalidState)?;
            if end > mmu.ram.len() || page.data.len() > PAGE_SIZE {
                return Err(SnapshotError::InvalidState)
            }
            mmu.ram[start..end].copy_from_slice(&page.data)
        }
        let snapshot = VmSnapshot {
            format_version: base.format_version,
            feature_flags: self.target_feature_flags,
            memory_size: self.target_memory_size,
            cpu: self.target_cpu,
            mmu,
            interrupt_controller: self.target_interrupt_controller.clone(),
            apic: self.target_apic.clone(),
            bios_state: self.target_bios_state,
            bios_ivt: self.target_bios_ivt,
            bios_bda: self.target_bios_bda,
            bios_ega: self.target_bios_ega,
            bios_reset_vector: self.target_bios_reset_vector,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

/// Parent-linked snapshot store. Only the first checkpoint owns full RAM;
/// later checkpoints retain page diffs and can be materialized on demand.
#[derive(Clone, Debug)]
pub struct SnapshotChain {
    base: VmSnapshot,
    records: Vec<(SnapshotId, SnapshotId, SnapshotDiff)>,
    next_id: SnapshotId,
}

impl SnapshotChain {
    pub fn new(base: VmSnapshot) -> Self {
        Self {
            base,
            records: Vec::new(),
            next_id: 1,
        }
    }

    pub fn base(&self) -> &VmSnapshot {
        &self.base
    }

    pub fn checkpoint(&mut self, snapshot: VmSnapshot) -> Result<SnapshotId, SnapshotError> {
        let parent = self.latest_id();
        let parent_snapshot = self.snapshot(parent)?;
        let diff = parent_snapshot.diff(&snapshot)?;
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.records.push((id, parent, diff));
        Ok(id)
    }

    pub fn latest_id(&self) -> SnapshotId {
        self.records.last().map(|record| record.0).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.records.len() + 1
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn snapshot(&self, id: SnapshotId) -> Result<VmSnapshot, SnapshotError> {
        if id == 0 {
            return Ok(self.base.clone())
        }
        let (_, parent, diff) = self
            .records
            .iter()
            .find(|record| record.0 == id)
            .ok_or(SnapshotError::MissingSnapshot(id))?;
        let parent_snapshot = self.snapshot(*parent)?;
        diff.apply_to(&parent_snapshot)
    }

    pub fn restore_into(&self, id: SnapshotId, vm: &mut Vm) -> Result<(), SnapshotError> {
        self.snapshot(id)?.restore_into(vm)
    }

    pub fn restore_into_with_report(
        &self,
        id: SnapshotId,
        vm: &mut Vm,
    ) -> Result<SnapshotRestoreReport, SnapshotError> {
        self.snapshot(id)?.restore_into_with_report(vm)
    }
}

fn constant_time_equal(left: &[u8; AUTH_TAG_BYTES], right: &[u8; AUTH_TAG_BYTES]) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn constant_time_equal_short(left: &[u8; 16], right: &[u8; 16]) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn hmac_sha256(key: &[u8; SNAPSHOT_AUTH_KEY_BYTES], parts: &[&[u8]]) -> [u8; AUTH_TAG_BYTES] {
    let mut normalized = [0u8; 64];
    normalized[..key.len()].copy_from_slice(key);

    let mut inner = Sha256::new();
    let inner_pad = normalized.map(|byte| byte ^ 0x36);
    inner.update(&inner_pad);
    for part in parts {
        inner.update(part)
    }
    let inner_hash = inner.finish();

    let mut outer = Sha256::new();
    let outer_pad = normalized.map(|byte| byte ^ 0x5c);
    outer.update(&outer_pad);
    outer.update(&inner_hash);
    outer.finish()
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667,
                0xbb67ae85,
                0x3c6ef372,
                0xa54ff53a,
                0x510e527f,
                0x9b05688c,
                0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            buffered: 0,
            length: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.buffered != 0 {
            let needed = 64 - self.buffered;
            if bytes.len() < needed {
                self.buffer[self.buffered..self.buffered + bytes.len()].copy_from_slice(bytes);
                self.buffered += bytes.len();
                return
            }
            self.buffer[self.buffered..].copy_from_slice(&bytes[..needed]);
            sha256_compress(&mut self.state, &self.buffer);
            self.buffered = 0;
            bytes = &bytes[needed..];
        }
        while bytes.len() >= 64 {
            sha256_compress(&mut self.state, &bytes[..64]);
            bytes = &bytes[64..];
        }
        self.buffer[..bytes.len()].copy_from_slice(bytes);
        self.buffered = bytes.len()
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length.wrapping_mul(8).to_be_bytes();
        self.buffer[self.buffered] = 0x80;
        self.buffered += 1;
        if self.buffered > 56 {
            self.buffer[self.buffered..].fill(0);
            sha256_compress(&mut self.state, &self.buffer);
            self.buffered = 0;
        }
        self.buffer[self.buffered..56].fill(0);
        self.buffer[56..].copy_from_slice(&bit_length);
        sha256_compress(&mut self.state, &self.buffer);

        let mut digest = [0u8; 32];
        for (bytes, word) in digest.chunks_exact_mut(4).zip(self.state) {
            bytes.copy_from_slice(&word.to_be_bytes())
        }
        digest
    }
}

fn sha256_compress(state: &mut [u32; 8], block: &[u8]) {
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354,
        0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c,
        0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f,
        0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut schedule = [0u32; 64];
    for (word, bytes) in schedule.iter_mut().zip(block.chunks_exact(4)).take(16) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    for index in 16..64 {
        let s0 = schedule[index - 15].rotate_right(7)
            ^ schedule[index - 15].rotate_right(18)
            ^ (schedule[index - 15] >> 3);
        let s1 = schedule[index - 2].rotate_right(17)
            ^ schedule[index - 2].rotate_right(19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1)
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let choice = (e & f) ^ (!e & g);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let first = h
            .wrapping_add(sum1)
            .wrapping_add(choice)
            .wrapping_add(ROUND[index])
            .wrapping_add(schedule[index]);
        let second = sum0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(first);
        d = c;
        c = b;
        b = a;
        a = first.wrapping_add(second)
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value)
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finish()
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x1000_0000_01b3)
    }
    value
}

fn encode_bios_state(state: BiosState) -> u8 {
    match state {
        BiosState::Reset => 0,
        BiosState::Initialized => 1,
        BiosState::UefiInitialized => 2,
        BiosState::Running => 3,
        BiosState::Halted => 4,
    }
}

fn decode_bios_state(value: u8) -> Result<BiosState, SnapshotError> {
    match value {
        0 => Ok(BiosState::Reset),
        1 => Ok(BiosState::Initialized),
        2 => Ok(BiosState::UefiInitialized),
        3 => Ok(BiosState::Running),
        4 => Ok(BiosState::Halted),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_cpu(writer: &mut Writer, state: &CpuState) {
    for value in [
        state.rax, state.rbx, state.rcx, state.rdx, state.rsi, state.rdi, state.rbp,
        state.rsp, state.r8, state.r9, state.r10, state.r11, state.r12, state.r13,
        state.r14, state.r15, state.rip, state.rflags, state.cr0, state.cr2, state.cr3,
        state.cr4, state.efer, state.ist_stack, state.star, state.lstar, state.fs_base,
        state.gs_base,
    ] {
        writer.u64(value)
    }
    for segment in [state.cs, state.ds, state.es, state.fs, state.gs, state.ss] {
        encode_segment(writer, &segment)
    }
    encode_descriptor(writer, &state.gdtr);
    encode_descriptor(writer, &state.idtr);
    writer.u8(encode_mode(state.mode));
    writer.u8(encode_privilege(state.privilege));
    writer.bool(state.halted);
    writer.bool(state.interrupt_shadow)
}

fn decode_cpu(reader: &mut Reader<'_>) -> Result<CpuState, SnapshotError> {
    let mut values = [0u64; 28];
    for value in &mut values {
        *value = reader.u64()?
    }
    let cs = decode_segment(reader)?;
    let ds = decode_segment(reader)?;
    let es = decode_segment(reader)?;
    let fs = decode_segment(reader)?;
    let gs = decode_segment(reader)?;
    let ss = decode_segment(reader)?;
    let gdtr = decode_descriptor(reader)?;
    let idtr = decode_descriptor(reader)?;
    let mode = decode_mode(reader.u8()?)?;
    let privilege = decode_privilege(reader.u8()?)?;
    let halted = reader.bool()?;
    let interrupt_shadow = reader.bool()?;
    Ok(CpuState {
        rax: values[0], rbx: values[1], rcx: values[2], rdx: values[3], rsi: values[4],
        rdi: values[5], rbp: values[6], rsp: values[7], r8: values[8], r9: values[9],
        r10: values[10], r11: values[11], r12: values[12], r13: values[13], r14: values[14],
        r15: values[15], rip: values[16], rflags: values[17], cr0: values[18], cr2: values[19],
        cr3: values[20], cr4: values[21], efer: values[22], ist_stack: values[23], star: values[24],
        lstar: values[25], fs_base: values[26], gs_base: values[27], cs, ds, es, fs, gs, ss,
        gdtr, idtr, mode, privilege, halted, interrupt_shadow,
    })
}

fn encode_segment(writer: &mut Writer, segment: &SegmentRegister) {
    writer.u16(segment.selector);
    writer.u64(segment.base);
    writer.u32(segment.limit);
    writer.u16(segment.attributes)
}

fn decode_segment(reader: &mut Reader<'_>) -> Result<SegmentRegister, SnapshotError> {
    Ok(SegmentRegister {
        selector: reader.u16()?,
        base: reader.u64()?,
        limit: reader.u32()?,
        attributes: reader.u16()?,
    })
}

fn encode_descriptor(writer: &mut Writer, descriptor: &DescriptorTableRegister) {
    writer.u64(descriptor.base);
    writer.u16(descriptor.limit)
}

fn decode_descriptor(reader: &mut Reader<'_>) -> Result<DescriptorTableRegister, SnapshotError> {
    Ok(DescriptorTableRegister {
        base: reader.u64()?,
        limit: reader.u16()?,
    })
}

fn encode_mode(mode: CpuMode) -> u8 {
    match mode {
        CpuMode::Real16 => 0,
        CpuMode::Protected16 => 1,
        CpuMode::Protected32 => 2,
        CpuMode::Long64 => 3,
    }
}

fn decode_mode(value: u8) -> Result<CpuMode, SnapshotError> {
    match value {
        0 => Ok(CpuMode::Real16),
        1 => Ok(CpuMode::Protected16),
        2 => Ok(CpuMode::Protected32),
        3 => Ok(CpuMode::Long64),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_privilege(privilege: PrivilegeLevel) -> u8 {
    match privilege {
        PrivilegeLevel::Ring0 => 0,
        PrivilegeLevel::Ring3 => 3,
    }
}

fn decode_privilege(value: u8) -> Result<PrivilegeLevel, SnapshotError> {
    match value {
        0 => Ok(PrivilegeLevel::Ring0),
        3 => Ok(PrivilegeLevel::Ring3),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_mmu(writer: &mut Writer, state: &MmuState) {
    writer.bytes_vec(&state.ram);
    writer.usize_vec(&state.free_frames);
    writer.u64(state.code_version);
    writer.u64_pairs(&state.identity);
    writer.u64_u64_u64_bool(&state.cow_pages);
    writer.u64_usize_pairs(&state.mapped_frames);
    writer.u64_pairs(&state.mapped_pages);
    writer.u64_vec(&state.ballooned_frames);
    writer.option_u64(state.zero_page);
    writer.usize(state.overcommitted_pages);
    writer.usize(state.overcommit_limit);
    writer.bool(state.paging_enabled);
    writer.u64(state.cr3);
    writer.bool(state.privilege)
}

fn decode_mmu(reader: &mut Reader<'_>, memory_size: usize) -> Result<MmuState, SnapshotError> {
    Ok(MmuState {
        ram: reader.bytes_vec(memory_size)?,
        free_frames: reader.usize_vec()?,
        code_version: reader.u64()?,
        identity: reader.u64_pairs()?,
        cow_pages: reader.u64_u64_u64_bool()?,
        mapped_frames: reader.u64_usize_pairs()?,
        mapped_pages: reader.u64_pairs()?,
        ballooned_frames: reader.u64_vec()?,
        zero_page: reader.option_u64()?,
        overcommitted_pages: reader.usize()?,
        overcommit_limit: reader.usize()?,
        paging_enabled: reader.bool()?,
        cr3: reader.u64()?,
        privilege: reader.bool()?,
    })
}

fn encode_interrupt_controller(writer: &mut Writer, state: &InterruptControllerState) {
    writer.u64(state.idt_base);
    writer.u16(state.idt_limit);
    writer.u64_pairs_u8(&state.irq_routing);
    writer.bool(state.pic_mapped)
}

fn decode_interrupt_controller(
    reader: &mut Reader<'_>,
) -> Result<InterruptControllerState, SnapshotError> {
    Ok(InterruptControllerState {
        idt_base: reader.u64()?,
        idt_limit: reader.u16()?,
        irq_routing: reader.u64_pairs_u8()?,
        pic_mapped: reader.bool()?,
    })
}

fn encode_apic(writer: &mut Writer, state: &LocalApicState) {
    writer.u64(state.base);
    writer.bool(state.enabled);
    for value in [
        state.id, state.version, state.tpr, state.ppr, state.ldr, state.dfr, state.svr,
    ] {
        writer.u32(value)
    }
    for table in [state.irr, state.isr, state.tmr, state.level_pending] {
        for value in table {
            writer.u32(value)
        }
    }
    for value in [
        state.esr, state.icr_hi, state.icr_lo, state.lvt_timer, state.lvt_thermal,
        state.lvt_perfmon, state.lvt_lint0, state.lvt_lint1, state.lvt_error,
        state.timer_initial_count, state.timer_current_count, state.timer_divide,
    ] {
        writer.u32(value)
    }
    writer.bool(state.timer_running);
    writer.option_u64(state.timer_last_ns);
    writer.bool(state.timer_fired)
}

fn decode_apic(reader: &mut Reader<'_>) -> Result<LocalApicState, SnapshotError> {
    let base = reader.u64()?;
    let enabled = reader.bool()?;
    let mut values = [0u32; 7];
    for value in &mut values {
        *value = reader.u32()?
    }
    let mut tables = [[0u32; 8]; 4];
    for table in &mut tables {
        for value in table {
            *value = reader.u32()?
        }
    }
    let mut device = [0u32; 12];
    for value in &mut device {
        *value = reader.u32()?
    }
    Ok(LocalApicState {
        base,
        enabled,
        id: values[0],
        version: values[1],
        tpr: values[2],
        ppr: values[3],
        ldr: values[4],
        dfr: values[5],
        svr: values[6],
        irr: tables[0],
        isr: tables[1],
        tmr: tables[2],
        level_pending: tables[3],
        esr: device[0],
        icr_hi: device[1],
        icr_lo: device[2],
        lvt_timer: device[3],
        lvt_thermal: device[4],
        lvt_perfmon: device[5],
        lvt_lint0: device[6],
        lvt_lint1: device[7],
        lvt_error: device[8],
        timer_initial_count: device[9],
        timer_current_count: device[10],
        timer_divide: device[11],
        timer_running: reader.bool()?,
        timer_last_ns: reader.option_u64()?,
        timer_fired: reader.bool()?,
    })
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value)
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value)
    }

    fn bool(&mut self, value: bool) {
        self.u8(value as u8)
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes())
    }

    fn usize(&mut self, value: usize) {
        self.u64(value as u64)
    }

    fn len(&mut self, value: usize) {
        self.usize(value)
    }

    fn bytes_vec(&mut self, value: &[u8]) {
        self.len(value.len());
        self.bytes(value)
    }

    fn usize_vec(&mut self, value: &[usize]) {
        self.len(value.len());
        for item in value {
            self.usize(*item)
        }
    }

    fn u64_vec(&mut self, value: &[u64]) {
        self.len(value.len());
        for item in value {
            self.u64(*item)
        }
    }

    fn u64_pairs(&mut self, value: &[(u64, u64)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u64(a);
            self.u64(b)
        }
    }

    fn u64_pairs_u8(&mut self, value: &[(u8, u64)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u8(a);
            self.u64(b)
        }
    }

    fn u64_usize_pairs(&mut self, value: &[(u64, usize)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u64(a);
            self.usize(b)
        }
    }

    fn u64_u64_u64_bool(&mut self, value: &[(u64, u64, u64, bool)]) {
        self.len(value.len());
        for &(a, b, c, d) in value {
            self.u64(a);
            self.u64(b);
            self.u64(c);
            self.bool(d)
        }
    }

    fn option_u64(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                self.bool(true);
                self.u64(value)
            }
            None => self.bool(false),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.offset.checked_add(len).ok_or(SnapshotError::InvalidFormat)?;
        if end > self.bytes.len() {
            return Err(SnapshotError::InvalidFormat)
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn bytes_exact(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        self.take(len)
    }

    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, SnapshotError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapshotError::InvalidFormat),
        }
    }

    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| SnapshotError::InvalidFormat)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, SnapshotError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| SnapshotError::InvalidFormat)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| SnapshotError::InvalidFormat)?,
        ))
    }

    fn usize(&mut self) -> Result<usize, SnapshotError> {
        usize::try_from(self.u64()?).map_err(|_| SnapshotError::InvalidFormat)
    }

    fn count_at_least(
        &mut self,
        minimum_bytes: usize,
        maximum_count: usize,
    ) -> Result<usize, SnapshotError> {
        let count = self.usize()?;
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if count > maximum_count || count > remaining / minimum_bytes.max(1) {
            return Err(SnapshotError::InvalidFormat)
        }
        Ok(count)
    }

    fn bytes_vec(&mut self, maximum_len: usize) -> Result<Vec<u8>, SnapshotError> {
        let len = self.count_at_least(1, maximum_len)?;
        Ok(self.take(len)?.to_vec())
    }

    fn usize_vec(&mut self) -> Result<Vec<usize>, SnapshotError> {
        let count = self.count_at_least(8, MAX_ITEMS)?;
        (0..count).map(|_| self.usize()).collect()
    }

    fn u64_vec(&mut self) -> Result<Vec<u64>, SnapshotError> {
        let count = self.count_at_least(8, MAX_ITEMS)?;
        (0..count).map(|_| self.u64()).collect()
    }

    fn u64_pairs(&mut self) -> Result<Vec<(u64, u64)>, SnapshotError> {
        let count = self.count_at_least(16, MAX_ITEMS)?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.u64()?)))
            .collect()
    }

    fn u64_pairs_u8(&mut self) -> Result<Vec<(u8, u64)>, SnapshotError> {
        let count = self.count_at_least(9, MAX_ITEMS)?;
        (0..count)
            .map(|_| Ok((self.u8()?, self.u64()?)))
            .collect()
    }

    fn u64_usize_pairs(&mut self) -> Result<Vec<(u64, usize)>, SnapshotError> {
        let count = self.count_at_least(16, MAX_ITEMS)?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.usize()?)))
            .collect()
    }

    fn u64_u64_u64_bool(&mut self) -> Result<Vec<(u64, u64, u64, bool)>, SnapshotError> {
        let count = self.count_at_least(25, MAX_ITEMS)?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.u64()?, self.u64()?, self.bool()?)))
            .collect()
    }

    fn option_u64(&mut self) -> Result<Option<u64>, SnapshotError> {
        if self.bool()? {
            Ok(Some(self.u64()?))
        } else {
            Ok(None)
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SnapshotError> {
        self.take(N)?.try_into().map_err(|_| SnapshotError::InvalidFormat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d,
                0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10,
                0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn schema_negotiation_selects_common_version_and_flags() {
        let negotiated = SnapshotSchema::negotiate(SnapshotSchema::for_version(1))
            .expect("v1 schema remains compatible");
        assert_eq!(negotiated.min_version, 1);
        assert_eq!(negotiated.max_version, 1);
        assert_eq!(negotiated.features, SnapshotFeatures::ALL);

        let missing = SnapshotSchema {
            min_version: 2,
            max_version: 2,
            features: SnapshotFeatures::CPU_STATE,
        };
        assert!(matches!(
            SnapshotSchema::negotiate(missing),
            Err(SnapshotError::MissingFeatures { .. })
        ));
    }

    #[test]
    fn v1_checkpoint_upgrades_to_v2_with_explicit_flags() {
        let vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        let v2 = VmSnapshot::capture(&vm);
        let v1 = v2
            .convert_to_schema(SnapshotSchema::for_version(1))
            .expect("convert to v1");
        let decoded_v1 = VmSnapshot::from_bytes(&v1.to_bytes()).expect("decode v1");
        assert_eq!(decoded_v1.format_version, 1);
        assert_eq!(decoded_v1.feature_flags, SnapshotFeatures::ALL);

        let upgraded = decoded_v1
            .convert_to_schema(SnapshotSchema::for_version(2))
            .expect("upgrade to v2");
        assert_eq!(upgraded.format_version, 2);
        assert_eq!(upgraded.feature_flags, SnapshotFeatures::ALL);
        assert_eq!(VmSnapshot::from_bytes(&upgraded.to_bytes()).expect("decode v2"), upgraded);
    }

    #[test]
    fn decoder_rejects_oversized_ram_and_unknown_flags() {
        let vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        let bytes = VmSnapshot::capture(&vm).to_bytes();

        let mut oversized = bytes.clone();
        oversized[20..28].copy_from_slice(&(MAX_SNAPSHOT_MEMORY_BYTES + 1).to_le_bytes());
        assert!(matches!(
            VmSnapshot::from_bytes(&oversized),
            Err(SnapshotError::SizeLimit { .. })
        ));

        let mut unknown = bytes;
        unknown[12..20].copy_from_slice(&(1u64 << 63).to_le_bytes());
        assert!(matches!(
            VmSnapshot::from_bytes(&unknown),
            Err(SnapshotError::UnsupportedFeatures { .. })
        ));
    }

    #[test]
    fn authenticated_snapshot_rejects_tampering_and_wrong_keys() {
        let vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        let snapshot = VmSnapshot::capture(&vm);
        let key = SnapshotAuthKey::new([0x42; SNAPSHOT_AUTH_KEY_BYTES]);
        let mut authenticated = snapshot
            .to_authenticated_bytes(key)
            .expect("authenticate snapshot");
        assert_eq!(
            VmSnapshot::from_authenticated_bytes(&authenticated, key).expect("verify snapshot"),
            snapshot
        );

        authenticated[AUTH_HEADER_BYTES] ^= 1;
        assert!(matches!(
            VmSnapshot::from_authenticated_bytes(&authenticated, key),
            Err(SnapshotError::AuthenticationFailed)
        ));
        let other_key = SnapshotAuthKey::new([0x24; SNAPSHOT_AUTH_KEY_BYTES]);
        assert!(matches!(
            VmSnapshot::from_authenticated_bytes(
                &snapshot.to_authenticated_bytes(key).expect("authenticate snapshot"),
                other_key
            ),
            Err(SnapshotError::AuthenticationFailed)
        ));
        assert!(matches!(
            VmSnapshot::from_authenticated_bytes(&snapshot.to_bytes(), key),
            Err(SnapshotError::AuthenticationRequired)
        ));
    }

    #[test]
    fn authenticated_snapshot_debug_and_wire_contain_only_key_identifier() {
        let key_bytes = [0x42; SNAPSHOT_AUTH_KEY_BYTES];
        let key = SnapshotAuthKey::new(key_bytes);
        let debug = format!("{key:?}");
        let secret_hex = "4242424242424242424242424242424242424242424242424242424242424242";
        assert!(!debug.contains(secret_hex));

        let vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        let bytes = VmSnapshot::capture(&vm)
            .to_authenticated_bytes(key)
            .expect("authenticate snapshot");
        assert_eq!(&bytes[16..32], &key.key_id());
        assert_ne!(&bytes[16..32], &key_bytes[..16]);
    }

    #[test]
    fn snapshot_restore_clears_translated_blocks_before_resume() {
        let mut vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        vm.mmu.write_phys(0x1000, &[0x90, 0xF4]).expect("snapshot code");
        vm.cpu.set_rip(0x1000);
        let snapshot = VmSnapshot::capture(&vm);

        vm.execution
            .execute(
                &mut vm.cpu,
                &mut vm.mmu,
                &mut vm.interrupt_controller,
                &mut vm.ports,
                &mut vm.bios.context,
                1,
            )
            .expect("populate translation cache");
        assert!(vm.execution.cache_len() > 0);

        vm.mmu.write_phys(0x1000, &[0xF4]).expect("mutate guest code");
        snapshot.restore_into(&mut vm).expect("restore snapshot");
        assert_eq!(vm.cpu.rip(), 0x1000);
        assert_eq!(vm.execution.cache_len(), 0);

        vm.execution
            .execute(
                &mut vm.cpu,
                &mut vm.mmu,
                &mut vm.interrupt_controller,
                &mut vm.ports,
                &mut vm.bios.context,
                1,
            )
            .expect("resume from restored code");
        assert!(!vm.cpu.state.halted);
    }

    #[test]
    fn snapshot_auth_magic_accepts_ghostos_alias() {
        assert!(auth_magic_accepted(AUTH_MAGIC));
        assert!(auth_magic_accepted(AUTH_MAGIC_GHOSTOS));
        assert!(!auth_magic_accepted(b"XXXXXXXX"));
    }

    #[test]
    fn authenticated_snapshot_accepts_ghostos_magic() {
        let vm = Vm::with_config(crate::VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..crate::VmConfig::default()
        });
        let snapshot = VmSnapshot::capture(&vm);
        let key = SnapshotAuthKey::new([0x42; SNAPSHOT_AUTH_KEY_BYTES]);
        let mut authenticated = snapshot
            .to_authenticated_bytes(key)
            .expect("authenticate snapshot");
        authenticated[..8].copy_from_slice(AUTH_MAGIC_GHOSTOS);
        let payload_end = authenticated.len() - AUTH_TAG_BYTES;
        let tag = key.authenticate_parts(&[&authenticated[..payload_end]]);
        authenticated[payload_end..].copy_from_slice(&tag);
        assert_eq!(
            VmSnapshot::from_authenticated_bytes(&authenticated, key).expect("verify ghostos magic"),
            snapshot
        );
    }
}
