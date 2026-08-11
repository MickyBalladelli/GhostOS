use synos_pkg::PackageDaemon;
use synos_system_model::ContentId;

use crate::{Error, Target, Text};

pub const MAX_REGISTRY_ENTRIES: usize = 64;
pub const MAX_REGISTRY_DEPENDENCIES: usize = 16;
pub const MAX_PACKAGE_NAME_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryEntry {
    pub name: Text<MAX_PACKAGE_NAME_BYTES>,
    pub version: Version,
    pub package: ContentId,
    pub target: Target,
    pub dependencies: [Option<ContentId>; MAX_REGISTRY_DEPENDENCIES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyPin {
    pub name: Text<MAX_PACKAGE_NAME_BYTES>,
    pub version: Version,
    pub package: ContentId,
    pub target: Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedDependency {
    pub ordinal: u16,
    pub pin: DependencyPin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyResolution {
    pub lockfile: ContentId,
    pub entries: [Option<ResolvedDependency>; MAX_REGISTRY_DEPENDENCIES],
    pub length: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyWave {
    pub entries: [Option<ResolvedDependency>; MAX_REGISTRY_DEPENDENCIES],
    pub length: u8,
}

impl DependencyWave {
    pub fn iter(&self) -> impl Iterator<Item = ResolvedDependency> + '_ {
        self.entries.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.length as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencySchedule {
    pub waves: [Option<DependencyWave>; MAX_REGISTRY_DEPENDENCIES],
    pub length: u8,
}

impl DependencySchedule {
    pub fn iter(&self) -> impl Iterator<Item = DependencyWave> + '_ {
        self.waves.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.length as usize
    }
}

impl DependencyResolution {
    pub fn iter(&self) -> impl Iterator<Item = ResolvedDependency> + '_ {
        self.entries.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.length as usize
    }

    /// Group independent packages into deterministic waves. A wave is safe to
    /// run in parallel; later waves wait only for packages in this lockfile.
    pub fn schedule(
        &self,
        registry: &SignedLocalRegistry,
    ) -> Result<DependencySchedule, RegistryError> {
        let mut completed = [false; MAX_REGISTRY_DEPENDENCIES];
        let mut completed_count = 0;
        let mut waves = [None; MAX_REGISTRY_DEPENDENCIES];
        let mut wave_count = 0;
        while completed_count < self.len() {
            let mut wave = DependencyWave {
                entries: [None; MAX_REGISTRY_DEPENDENCIES],
                length: 0,
            };
            for index in 0..self.len() {
                if completed[index] {
                    continue
                }
                let dependency = self.entries[index].ok_or(RegistryError::InvalidDependency)?;
                let entry = registry
                    .entries()
                    .find(|entry| entry.package == dependency.pin.package)
                    .ok_or(RegistryError::PackageNotFound)?;
                let blocked = entry.dependencies.iter().flatten().any(|package| {
                    self.entries
                        .iter()
                        .take(self.len())
                        .enumerate()
                        .any(|(candidate, resolved)| {
                            resolved.is_some_and(|resolved| {
                                resolved.pin.package == *package && !completed[candidate]
                            })
                        })
                });
                if blocked {
                    continue
                }
                wave.entries[wave.length as usize] = Some(dependency);
                wave.length += 1;
            }
            if wave.length == 0 {
                return Err(RegistryError::DependencyCycle)
            }
            for dependency in wave.iter() {
                if let Some(index) = self.entries.iter().position(|entry| {
                    entry.is_some_and(|entry| entry.pin.package == dependency.pin.package)
                }) {
                    completed[index] = true;
                    completed_count += 1;
                }
            }
            waves[wave_count] = Some(wave);
            wave_count += 1;
        }
        Ok(DependencySchedule {
            waves,
            length: wave_count as u8,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    InvalidRegistry,
    Capacity,
    Duplicate,
    PackageNotAuthorized,
    PackageNotFound,
    MissingDependency,
    LockfileRequired,
    LockfileMismatch,
    InvalidDependency,
    DependencyCycle,
}

impl From<Error> for RegistryError {
    fn from(_: Error) -> Self {
        Self::InvalidDependency
    }
}

/// Immutable-name index for packages already verified by `synos-pkg`.
/// Registry records never authorize a package by themselves: the package
/// daemon must have verified and installed the signed package first.
pub struct SignedLocalRegistry {
    registry_package: ContentId,
    entries: [Option<RegistryEntry>; MAX_REGISTRY_ENTRIES],
}

impl SignedLocalRegistry {
    pub const fn new(registry_package: ContentId) -> Result<Self, RegistryError> {
        if registry_package.is_zero() {
            return Err(RegistryError::InvalidRegistry);
        }
        Ok(Self {
            registry_package,
            entries: [None; MAX_REGISTRY_ENTRIES],
        })
    }

    pub const fn package(&self) -> ContentId {
        self.registry_package
    }

    pub fn new_authorized<const PACKAGES: usize, const KEYS: usize>(
        packages: &PackageDaemon<PACKAGES, KEYS>,
        registry_package: ContentId,
    ) -> Result<Self, RegistryError> {
        packages
            .authorize_instantiation(registry_package)
            .map_err(|_| RegistryError::PackageNotAuthorized)?;
        Self::new(registry_package)
    }

    pub fn install_authorized<const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        entry: RegistryEntry,
    ) -> Result<(), RegistryError> {
        if entry.package.is_zero()
            || entry.name.as_str().is_empty()
            || entry.dependencies.iter().flatten().any(|dependency| dependency.is_zero())
        {
            return Err(RegistryError::InvalidDependency);
        }
        packages
            .authorize_instantiation(entry.package)
            .map_err(|_| RegistryError::PackageNotAuthorized)?;
        let manifest = packages
            .manifest(entry.package)
            .ok_or(RegistryError::PackageNotFound)?;
        if entry.dependencies.iter().flatten().any(|dependency| {
            !packages.is_instantiation_authorized(*dependency)
        }) {
            return Err(RegistryError::MissingDependency);
        }
        if self.entries.iter().flatten().any(|current| {
            current.name == entry.name && current.version == entry.version && current.target == entry.target
        }) {
            return Err(RegistryError::Duplicate);
        }
        if manifest.content != entry.package {
            return Err(RegistryError::PackageNotFound);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistryError::Capacity)?;
        *slot = Some(entry);
        Ok(())
    }

    pub fn resolve(
        &self,
        name: Text<MAX_PACKAGE_NAME_BYTES>,
        version: Version,
        target: Target,
    ) -> Result<RegistryEntry, RegistryError> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.name == name && entry.version == version && entry.target == target)
            .copied()
            .ok_or(RegistryError::PackageNotFound)
    }

    pub fn resolve_locked(
        &self,
        lockfile: ContentId,
        pins: &[DependencyPin; MAX_REGISTRY_DEPENDENCIES],
        length: usize,
    ) -> Result<DependencyResolution, RegistryError> {
        if lockfile.is_zero() {
            return Err(RegistryError::LockfileRequired);
        }
        if length > MAX_REGISTRY_DEPENDENCIES {
            return Err(RegistryError::Capacity);
        }
        let expected = lockfile_digest(pins, length);
        if expected != lockfile {
            return Err(RegistryError::LockfileMismatch);
        }
        let mut entries = [None; MAX_REGISTRY_DEPENDENCIES];
        for (ordinal, pin) in pins[..length].iter().copied().enumerate() {
            let entry = self.resolve(pin.name, pin.version, pin.target)?;
            if entry.package != pin.package {
                return Err(RegistryError::LockfileMismatch);
            }
            entries[ordinal] = Some(ResolvedDependency {
                ordinal: ordinal as u16,
                pin,
            });
        }
        Ok(DependencyResolution {
            lockfile,
            entries,
            length: length as u16,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = RegistryEntry> + '_ {
        self.entries.iter().flatten().copied()
    }
}

pub fn lockfile_digest(
    pins: &[DependencyPin; MAX_REGISTRY_DEPENDENCIES],
    length: usize,
) -> ContentId {
    let mut material = [0u8; MAX_REGISTRY_DEPENDENCIES * (MAX_PACKAGE_NAME_BYTES + 42)];
    let mut cursor = 0;
    for pin in pins.iter().take(length) {
        let name = pin.name.as_str().as_bytes();
        if cursor + name.len() + 42 > material.len() {
            break;
        }
        material[cursor..cursor + name.len()].copy_from_slice(name);
        cursor += name.len();
        material[cursor..cursor + 2].copy_from_slice(&pin.version.major.to_be_bytes());
        cursor += 2;
        material[cursor..cursor + 2].copy_from_slice(&pin.version.minor.to_be_bytes());
        cursor += 2;
        material[cursor..cursor + 2].copy_from_slice(&pin.version.patch.to_be_bytes());
        cursor += 2;
        material[cursor..cursor + 32].copy_from_slice(pin.package.as_bytes());
        cursor += 32;
        material[cursor] = pin.target as u8;
        cursor += 1;
        material[cursor] = 0;
        cursor += 1;
    }
    ContentId::hash(&material[..cursor])
}
