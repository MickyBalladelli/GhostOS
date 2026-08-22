use crate::{BuildResult, CacheKey, Error, Target};
use ghostos_system_model::ContentId;

pub const MAX_SHARED_ARTIFACTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedArtifact {
    pub package: ContentId,
    pub payload: ContentId,
    pub target: Target,
    pub leases: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedArtifactError {
    Capacity,
    InvalidArtifact,
    NotFound,
    Conflict,
}

impl From<SharedArtifactError> for Error {
    fn from(error: SharedArtifactError) -> Self {
        match error {
            SharedArtifactError::Capacity => Self::Capacity,
            SharedArtifactError::InvalidArtifact
            | SharedArtifactError::NotFound
            | SharedArtifactError::Conflict => Self::InvalidRequest,
        }
    }
}

pub struct SharedImmutableArtifacts<const CAPACITY: usize = MAX_SHARED_ARTIFACTS> {
    entries: [Option<SharedArtifact>; CAPACITY],
}

impl<const CAPACITY: usize> SharedImmutableArtifacts<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
        }
    }

    pub fn can_publish(&self, result: BuildResult) -> Result<(), SharedArtifactError> {
        validate_result(result)?;
        if self.entries.iter().flatten().any(|entry| {
            entry.package == result.package && entry.payload != result.payload
                || entry.payload == result.payload && entry.package != result.package
                || entry.package == result.package
                    && entry.payload == result.payload
                    && entry.target != result.target
        }) {
            return Err(SharedArtifactError::Conflict);
        }
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| {
                entry.package == result.package
                    && entry.payload == result.payload
                    && entry.target == result.target
            })
        {
            return Ok(())
        }
        if self.entries.iter().any(Option::is_none) {
            Ok(())
        } else {
            Err(SharedArtifactError::Capacity)
        }
    }

    pub fn publish(&mut self, result: BuildResult) -> Result<(), SharedArtifactError> {
        self.can_publish(result)?;
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.package == result.package && entry.payload == result.payload)
        {
            return Ok(())
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SharedArtifactError::Capacity)?;
        *slot = Some(SharedArtifact {
            package: result.package,
            payload: result.payload,
            target: result.target,
            leases: 0,
        });
        Ok(())
    }

    pub fn contains(&self, result: BuildResult) -> bool {
        self.entries.iter().flatten().any(|entry| {
            entry.package == result.package
                && entry.payload == result.payload
                && entry.target == result.target
        })
    }

    pub fn acquire(&mut self, result: BuildResult) -> Result<(), SharedArtifactError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| {
                entry.package == result.package
                    && entry.payload == result.payload
                    && entry.target == result.target
            })
            .ok_or(SharedArtifactError::NotFound)?;
        entry.leases = entry
            .leases
            .checked_add(1)
            .ok_or(SharedArtifactError::Capacity)?;
        Ok(())
    }

    pub fn release(&mut self, result: BuildResult) -> Result<(), SharedArtifactError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| {
                entry.package == result.package
                    && entry.payload == result.payload
                    && entry.target == result.target
            })
            .ok_or(SharedArtifactError::NotFound)?;
        entry.leases = entry.leases.saturating_sub(1);
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = SharedArtifact> + '_ {
        self.entries.iter().flatten().copied()
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }
}

impl<const CAPACITY: usize> Default for SharedImmutableArtifacts<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteCacheEntry {
    pub key: CacheKey,
    pub result: BuildResult,
    pub proof: ContentId,
}

impl RemoteCacheEntry {
    pub fn new(key: CacheKey, result: BuildResult) -> Result<Self, SharedArtifactError> {
        validate_result(result)?;
        if key.target != result.target
            || key.source.is_zero()
            || key.lockfile.is_zero()
            || key.toolchain.is_zero()
        {
            return Err(SharedArtifactError::InvalidArtifact);
        }
        Ok(Self {
            key,
            result,
            proof: cache_proof(key, result),
        })
    }

    pub fn validate<E>(self, expected: CacheKey) -> Result<BuildResult, RemoteCacheError<E>> {
        if self.key != expected
            || self.key.target != self.result.target
            || self.key.source.is_zero()
            || self.key.lockfile.is_zero()
            || self.key.toolchain.is_zero()
            || self.proof != cache_proof(self.key, self.result)
        {
            return Err(RemoteCacheError::InvalidEntry);
        }
        validate_result(self.result).map_err(|_| RemoteCacheError::InvalidEntry)?;
        Ok(self.result)
    }
}

pub trait RemoteCache {
    type Error;

    fn lookup(&mut self, key: CacheKey) -> Result<Option<RemoteCacheEntry>, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteCacheError<E = ()> {
    Backend(E),
    InvalidEntry,
    Capacity,
}

fn validate_result(result: BuildResult) -> Result<(), SharedArtifactError> {
    if result.package.is_zero() || result.payload.is_zero() {
        return Err(SharedArtifactError::InvalidArtifact);
    }
    Ok(())
}

fn cache_proof(key: CacheKey, result: BuildResult) -> ContentId {
    let mut material = [0; 32 * 6 + 3];
    let mut cursor = 0;
    for id in [key.source, key.lockfile, key.toolchain, key.features, result.package, result.payload] {
        material[cursor..cursor + 32].copy_from_slice(id.as_bytes());
        cursor += 32;
    }
    material[cursor] = key.target as u8;
    material[cursor + 1] = key.profile as u8;
    material[cursor + 2] = result.target as u8;
    ContentId::hash(&material)
}
