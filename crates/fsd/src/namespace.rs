use host_filesystems::{FileSystemKind, Partition};

pub const MAX_NAMESPACE_PATH_BYTES: usize = 96;
pub const DEFAULT_NAMESPACE_MOUNTS: usize = 16;

pub const ROOT_PATH: &str = "/";
pub const PACKAGE_STORE_PATH: &str = "/packages";
pub const LOGS_PATH: &str = "/logs";
pub const USER_DATA_PATH: &str = "/data";
pub const TEMPORARY_PATH: &str = "/tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceError {
    AlreadyActive,
    Inactive,
    Capacity,
    InvalidPath,
    InvalidPartition,
    AlreadyMounted,
    NotFound,
    InvalidCapability,
    AccessDenied,
    RootBusy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespacePath {
    bytes: [u8; MAX_NAMESPACE_PATH_BYTES],
    length: u8,
}

impl NamespacePath {
    pub fn new(path: &str) -> Result<Self, NamespaceError> {
        let bytes = path.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_NAMESPACE_PATH_BYTES
            || bytes[0] != b'/'
            || bytes.contains(&0)
            || bytes.contains(&b'\\')
            || bytes.windows(2).any(|pair| pair == b"//")
            || bytes
                .split(|byte| *byte == b'/')
                .any(|component| component == b"." || component == b"..")
            || (bytes.len() > 1 && bytes.ends_with(b"/"))
        {
            return Err(NamespaceError::InvalidPath);
        }
        core::str::from_utf8(bytes).map_err(|_| NamespaceError::InvalidPath)?;
        let mut stored = [0; MAX_NAMESPACE_PATH_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            length: bytes.len() as u8,
        })
    }

    pub const fn root() -> Self {
        let mut bytes = [0; MAX_NAMESPACE_PATH_BYTES];
        bytes[0] = b'/';
        Self { bytes, length: 1 }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize]).expect("NamespacePath invariant")
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }

    fn contains(self, path: &Self) -> bool {
        self == Self::root()
            || path == &self
            || path
                .as_str()
                .strip_prefix(self.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynFsVolume {
    Root,
    PackageStore,
    Logs,
    UserData,
    Temporary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountSource {
    SynFs {
        volume: SynFsVolume,
        generation: u64,
    },
    Host {
        filesystem: FileSystemKind,
        partition: Partition,
    },
}

impl MountSource {
    pub const fn is_host(self) -> bool {
        matches!(self, Self::Host { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NamespaceMountId(u32);

impl NamespaceMountId {
    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct MountCapability(u64);

impl MountCapability {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 || raw as u32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostMountAuthority(u64);

impl HostMountAuthority {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountInfo {
    pub id: NamespaceMountId,
    pub path: NamespacePath,
    pub source: MountSource,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MountSlot {
    occupied: bool,
    generation: u32,
    info: MountInfo,
}

impl MountSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        info: MountInfo {
            id: NamespaceMountId(0),
            path: NamespacePath::root(),
            source: MountSource::SynFs {
                volume: SynFsVolume::Root,
                generation: 0,
            },
            read_only: true,
        },
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootActivation {
    pub generation: u64,
    pub host_mount_authority: HostMountAuthority,
}

pub struct RootFilesystem;

impl RootFilesystem {
    pub const fn new() -> Self {
        Self
    }

    pub fn activate<const MAX_MOUNTS: usize>(
        &self,
        namespace: &mut Namespace<MAX_MOUNTS>,
        generation: u64,
    ) -> Result<RootActivation, NamespaceError> {
        namespace.activate_root(generation)
    }
}

impl Default for RootFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Namespace<const MAX_MOUNTS: usize = DEFAULT_NAMESPACE_MOUNTS> {
    active: bool,
    mounts: [MountSlot; MAX_MOUNTS],
    next_mount_id: u32,
    host_mount_authority: HostMountAuthority,
}

impl<const MAX_MOUNTS: usize> Namespace<MAX_MOUNTS> {
    pub const fn new() -> Self {
        Self {
            active: false,
            mounts: [MountSlot::EMPTY; MAX_MOUNTS],
            next_mount_id: 1,
            host_mount_authority: HostMountAuthority(0),
        }
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    pub fn activate_root(&mut self, generation: u64) -> Result<RootActivation, NamespaceError> {
        if self.active {
            return Err(NamespaceError::AlreadyActive);
        }
        if MAX_MOUNTS < 5 {
            return Err(NamespaceError::Capacity);
        }
        self.host_mount_authority = HostMountAuthority(0x4e53_5041_4345_0001);
        self.install(
            ROOT_PATH,
            MountSource::SynFs {
                volume: SynFsVolume::Root,
                generation,
            },
            true,
        )?;
        self.install(
            PACKAGE_STORE_PATH,
            MountSource::SynFs {
                volume: SynFsVolume::PackageStore,
                generation,
            },
            true,
        )?;
        self.install(
            LOGS_PATH,
            MountSource::SynFs {
                volume: SynFsVolume::Logs,
                generation,
            },
            false,
        )?;
        self.install(
            USER_DATA_PATH,
            MountSource::SynFs {
                volume: SynFsVolume::UserData,
                generation,
            },
            false,
        )?;
        self.install(
            TEMPORARY_PATH,
            MountSource::SynFs {
                volume: SynFsVolume::Temporary,
                generation,
            },
            false,
        )?;
        self.active = true;
        Ok(RootActivation {
            generation,
            host_mount_authority: self.host_mount_authority,
        })
    }

    pub fn mount_host(
        &mut self,
        authority: HostMountAuthority,
        path: &str,
        filesystem: FileSystemKind,
        partition: Partition,
    ) -> Result<(MountInfo, MountCapability), NamespaceError> {
        if !self.active {
            return Err(NamespaceError::Inactive);
        }
        if authority != self.host_mount_authority {
            return Err(NamespaceError::AccessDenied);
        }
        if partition.length == 0 || partition.start.checked_add(partition.length).is_none() {
            return Err(NamespaceError::InvalidPartition);
        }
        let path = NamespacePath::new(path)?;
        if self
            .mounts
            .iter()
            .any(|slot| slot.occupied && slot.info.path == path)
        {
            return Err(NamespaceError::AlreadyMounted);
        }
        let (index, slot) = self
            .mounts
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(NamespaceError::Capacity)?;
        let info = Self::install_in_slot(
            &mut self.next_mount_id,
            slot,
            path,
            MountSource::Host {
                filesystem,
                partition,
            },
            true,
        );
        Ok((info, self.capability(index)))
    }

    pub fn unmount(&mut self, capability: MountCapability) -> Result<(), NamespaceError> {
        let index = self.mount_index(capability)?;
        if index < 5 {
            return Err(NamespaceError::RootBusy);
        }
        self.mounts[index].occupied = false;
        Ok(())
    }

    pub fn resolve(&self, path: &str) -> Result<MountInfo, NamespaceError> {
        if !self.active {
            return Err(NamespaceError::Inactive);
        }
        let path = NamespacePath::new(path)?;
        self.mounts
            .iter()
            .filter(|slot| slot.occupied && slot.info.path.contains(&path))
            .max_by_key(|slot| slot.info.path.as_bytes().len())
            .map(|slot| slot.info)
            .ok_or(NamespaceError::NotFound)
    }

    pub fn mount(&self, capability: MountCapability) -> Result<MountInfo, NamespaceError> {
        let index = self.mount_index(capability)?;
        Ok(self.mounts[index].info)
    }

    pub fn mounts(&self) -> impl Iterator<Item = MountInfo> + '_ {
        self.mounts
            .iter()
            .filter(|slot| slot.occupied)
            .map(|slot| slot.info)
    }

    pub fn is_read_only(&self, path: &str) -> Result<bool, NamespaceError> {
        Ok(self.resolve(path)?.read_only)
    }

    fn install(
        &mut self,
        path: &str,
        source: MountSource,
        read_only: bool,
    ) -> Result<MountInfo, NamespaceError> {
        let path = NamespacePath::new(path)?;
        let index = self
            .mounts
            .iter()
            .position(|slot| !slot.occupied)
            .ok_or(NamespaceError::Capacity)?;
        let mut slot = self.mounts[index];
        let info =
            Self::install_in_slot(&mut self.next_mount_id, &mut slot, path, source, read_only);
        self.mounts[index] = slot;
        Ok(info)
    }

    fn install_in_slot(
        next_mount_id: &mut u32,
        slot: &mut MountSlot,
        path: NamespacePath,
        source: MountSource,
        read_only: bool,
    ) -> MountInfo {
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.info = MountInfo {
            id: NamespaceMountId(*next_mount_id),
            path,
            source,
            read_only,
        };
        *next_mount_id = next_mount_id.wrapping_add(1).max(1);
        slot.info
    }

    fn mount_index(&self, capability: MountCapability) -> Result<usize, NamespaceError> {
        let index = (capability.0 as u32)
            .checked_sub(1)
            .map(|value| value as usize)
            .ok_or(NamespaceError::InvalidCapability)?;
        let slot = self
            .mounts
            .get(index)
            .ok_or(NamespaceError::InvalidCapability)?;
        if !slot.occupied || slot.generation != (capability.0 >> 32) as u32 {
            return Err(NamespaceError::InvalidCapability);
        }
        Ok(index)
    }

    fn capability(&self, index: usize) -> MountCapability {
        MountCapability(((self.mounts[index].generation as u64) << 32) | (index as u64 + 1))
    }
}

impl<const MAX_MOUNTS: usize> Default for Namespace<MAX_MOUNTS> {
    fn default() -> Self {
        Self::new()
    }
}
