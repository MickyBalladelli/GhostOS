#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use synos_synfs::{Error as SynFsError, SynFs};

pub const MAX_NAME_BYTES: usize = 64;
pub const MAX_DEPENDENCIES: usize = 8;
pub const DEFAULT_PACKAGE_CAPACITY: usize = 128;
pub const DEFAULT_ROOT_BINDINGS: usize = 64;
const PACKAGE_IDENTITY_BYTES: usize = 313;
const ROOT_SNAPSHOT_BYTES: usize = 18 + DEFAULT_ROOT_BINDINGS * (1 + MAX_NAME_BYTES + 32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyBound,
    EmptyRoot,
    InvalidEntryPoint,
    InvalidName,
    MissingDependency,
    PackageNotFound,
    StaleRevision,
    StoreFull,
    TooManyDependencies,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryError {
    CorruptObject,
    Model(Error),
    SynFs(SynFsError),
    VerificationBufferTooSmall { required: usize },
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct ContentId([u8; 32]);

impl ContentId {
    pub fn hash(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LogicalName {
    bytes: [u8; MAX_NAME_BYTES],
    len: u8,
}

impl LogicalName {
    pub fn new(name: &str) -> Result<Self, Error> {
        let bytes = name.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_NAME_BYTES
            || !bytes[0].is_ascii_alphabetic()
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(Error::InvalidName);
        }

        let mut stored = [0; MAX_NAME_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("LogicalName invariant")
    }
}

impl fmt::Debug for LogicalName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LogicalName")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageManifest {
    /// Digest of the complete package manifest and dependency closure.
    pub content: ContentId,
    /// Digest used to locate the immutable payload object in SynFS.
    pub payload: ContentId,
    pub byte_length: u64,
    pub entry_offset: u64,
    dependencies: [Option<ContentId>; MAX_DEPENDENCIES],
}

impl PackageManifest {
    pub fn new(
        payload: &[u8],
        entry_offset: u64,
        dependencies: &[ContentId],
    ) -> Result<Self, Error> {
        if entry_offset > payload.len() as u64 {
            return Err(Error::InvalidEntryPoint);
        }
        if dependencies.len() > MAX_DEPENDENCIES {
            return Err(Error::TooManyDependencies);
        }

        let mut stored_dependencies = [None; MAX_DEPENDENCIES];
        for (slot, dependency) in stored_dependencies.iter_mut().zip(dependencies) {
            *slot = Some(*dependency)
        }

        let payload_id = ContentId::hash(payload);
        let (identity, identity_length) = encode_package_identity(
            payload_id,
            payload.len() as u64,
            entry_offset,
            &stored_dependencies,
        );

        Ok(Self {
            content: ContentId::hash(&identity[..identity_length]),
            payload: payload_id,
            byte_length: payload.len() as u64,
            entry_offset,
            dependencies: stored_dependencies,
        })
    }

    pub fn dependencies(&self) -> impl Iterator<Item = ContentId> + '_ {
        self.dependencies.iter().flatten().copied()
    }
}

/// Metadata index for immutable package payloads stored in SynFS by digest.
///
/// Installing the same bytes is idempotent. Dependencies are digest-pinned,
/// so package resolution never reads ambient paths or mutable global state.
pub struct PackageStore<const CAPACITY: usize = DEFAULT_PACKAGE_CAPACITY> {
    packages: [Option<PackageManifest>; CAPACITY],
}

impl<const CAPACITY: usize> PackageStore<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            packages: [None; CAPACITY],
        }
    }

    pub fn install(&mut self, package: PackageManifest) -> Result<ContentId, Error> {
        if self.validate_install(&package)? {
            return Ok(package.content);
        }

        let slot = self
            .packages
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::StoreFull)?;
        *slot = Some(package);
        Ok(package.content)
    }

    fn validate_install(&self, package: &PackageManifest) -> Result<bool, Error> {
        if self.get(package.content).is_some() {
            return Ok(true);
        }
        if package
            .dependencies()
            .any(|dependency| self.get(dependency).is_none())
        {
            return Err(Error::MissingDependency);
        }
        if self.packages.iter().all(Option::is_some) {
            return Err(Error::StoreFull);
        }
        Ok(false)
    }

    pub fn get(&self, content: ContentId) -> Option<&PackageManifest> {
        self.packages
            .iter()
            .flatten()
            .find(|package| package.content == content)
    }

    pub fn contains(&self, content: ContentId) -> bool {
        self.get(content).is_some()
    }
}

impl<const CAPACITY: usize> Default for PackageStore<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootBinding {
    pub name: LogicalName,
    pub package: ContentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootManifest<const CAPACITY: usize = DEFAULT_ROOT_BINDINGS> {
    revision: u64,
    bindings: [Option<RootBinding>; CAPACITY],
}

impl<const CAPACITY: usize> RootManifest<CAPACITY> {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn bindings(&self) -> impl Iterator<Item = RootBinding> + '_ {
        self.bindings.iter().flatten().copied()
    }

    pub fn resolve(&self, name: LogicalName) -> Option<ContentId> {
        self.bindings()
            .find(|binding| binding.name == name)
            .map(|binding| binding.package)
    }
}

pub struct RootBuilder<const CAPACITY: usize = DEFAULT_ROOT_BINDINGS> {
    revision: u64,
    bindings: [Option<RootBinding>; CAPACITY],
}

impl<const CAPACITY: usize> RootBuilder<CAPACITY> {
    pub const fn new(revision: u64) -> Self {
        Self {
            revision,
            bindings: [None; CAPACITY],
        }
    }

    pub fn bind(&mut self, name: LogicalName, package: ContentId) -> Result<(), Error> {
        if self
            .bindings
            .iter()
            .flatten()
            .any(|binding| binding.name == name)
        {
            return Err(Error::AlreadyBound);
        }
        let slot = self
            .bindings
            .iter_mut()
            .find(|binding| binding.is_none())
            .ok_or(Error::StoreFull)?;
        *slot = Some(RootBinding { name, package });
        Ok(())
    }

    pub fn seal(self) -> Result<RootManifest<CAPACITY>, Error> {
        if self.bindings.iter().all(Option::is_none) {
            return Err(Error::EmptyRoot);
        }
        Ok(RootManifest {
            revision: self.revision,
            bindings: self.bindings,
        })
    }
}

/// The only mutable root pointer. A validated manifest replaces it in one move.
pub struct RootController<const CAPACITY: usize = DEFAULT_ROOT_BINDINGS> {
    active: Option<RootManifest<CAPACITY>>,
}

impl<const CAPACITY: usize> RootController<CAPACITY> {
    pub const fn new() -> Self {
        Self { active: None }
    }

    pub const fn active(&self) -> Option<&RootManifest<CAPACITY>> {
        self.active.as_ref()
    }

    pub fn activate<const PACKAGES: usize>(
        &mut self,
        manifest: RootManifest<CAPACITY>,
        store: &PackageStore<PACKAGES>,
    ) -> Result<Option<RootManifest<CAPACITY>>, Error> {
        self.validate(&manifest, store)?;
        Ok(self.active.replace(manifest))
    }

    fn validate<const PACKAGES: usize>(
        &self,
        manifest: &RootManifest<CAPACITY>,
        store: &PackageStore<PACKAGES>,
    ) -> Result<(), Error> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| manifest.revision() <= active.revision())
        {
            return Err(Error::StaleRevision);
        }
        if manifest
            .bindings()
            .any(|binding| !store.contains(binding.package))
        {
            return Err(Error::PackageNotFound);
        }
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for RootController<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-process package view. Nothing outside this digest allowlist resolves.
pub struct PackageNamespace<const CAPACITY: usize = DEFAULT_ROOT_BINDINGS> {
    allowed: [Option<ContentId>; CAPACITY],
}

impl<const CAPACITY: usize> PackageNamespace<CAPACITY> {
    pub const fn empty() -> Self {
        Self {
            allowed: [None; CAPACITY],
        }
    }

    pub fn allow<const PACKAGES: usize>(
        &mut self,
        content: ContentId,
        store: &PackageStore<PACKAGES>,
    ) -> Result<(), Error> {
        if !store.contains(content) {
            return Err(Error::PackageNotFound);
        }
        if self.contains(content) {
            return Ok(());
        }
        let slot = self
            .allowed
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::StoreFull)?;
        *slot = Some(content);
        Ok(())
    }

    pub fn contains(&self, content: ContentId) -> bool {
        self.allowed
            .iter()
            .flatten()
            .any(|allowed| *allowed == content)
    }

    pub fn resolve<'a, const PACKAGES: usize>(
        &self,
        content: ContentId,
        store: &'a PackageStore<PACKAGES>,
    ) -> Result<&'a PackageManifest, Error> {
        if !self.contains(content) {
            return Err(Error::PackageNotFound);
        }
        store.get(content).ok_or(Error::PackageNotFound)
    }
}

impl<const CAPACITY: usize> Default for PackageNamespace<CAPACITY> {
    fn default() -> Self {
        Self::empty()
    }
}

/// Persistent system repository backed by immutable, versioned SynFS objects.
pub struct SynFsRepository<const PACKAGES: usize = DEFAULT_PACKAGE_CAPACITY> {
    packages: PackageStore<PACKAGES>,
    root: RootController<DEFAULT_ROOT_BINDINGS>,
}

impl<const PACKAGES: usize> SynFsRepository<PACKAGES> {
    pub const fn new() -> Self {
        Self {
            packages: PackageStore::new(),
            root: RootController::new(),
        }
    }

    pub const fn packages(&self) -> &PackageStore<PACKAGES> {
        &self.packages
    }

    pub const fn active_root(&self) -> Option<&RootManifest<DEFAULT_ROOT_BINDINGS>> {
        self.root.active()
    }

    pub fn install<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        payload: &[u8],
        entry_offset: u64,
        dependencies: &[ContentId],
        verification_buffer: &mut [u8],
    ) -> Result<ContentId, RepositoryError> {
        let manifest = PackageManifest::new(payload, entry_offset, dependencies)
            .map_err(RepositoryError::Model)?;
        self.packages
            .validate_install(&manifest)
            .map_err(RepositoryError::Model)?;

        let payload_path = ObjectPath::new("system/store/sha256-", manifest.payload);
        persist_object(fs, payload_path.as_str(), payload, verification_buffer)?;

        let manifest_path = ObjectPath::new("system/manifests/sha256-", manifest.content);
        let (identity, identity_length) = encode_package_identity(
            manifest.payload,
            manifest.byte_length,
            manifest.entry_offset,
            &manifest.dependencies,
        );
        persist_object(
            fs,
            manifest_path.as_str(),
            &identity[..identity_length],
            verification_buffer,
        )?;
        self.packages
            .install(manifest)
            .map_err(RepositoryError::Model)?;
        Ok(manifest.content)
    }

    pub fn activate<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        manifest: RootManifest<DEFAULT_ROOT_BINDINGS>,
    ) -> Result<Option<RootManifest<DEFAULT_ROOT_BINDINGS>>, RepositoryError> {
        self.root
            .validate(&manifest, &self.packages)
            .map_err(RepositoryError::Model)?;
        let (snapshot, snapshot_length) = encode_root_snapshot(&manifest);
        fs.write("system/root.manifest", &snapshot[..snapshot_length])
            .map_err(RepositoryError::SynFs)?;
        self.root
            .activate(manifest, &self.packages)
            .map_err(RepositoryError::Model)
    }
}

impl<const PACKAGES: usize> Default for SynFsRepository<PACKAGES> {
    fn default() -> Self {
        Self::new()
    }
}

struct ObjectPath {
    bytes: [u8; 96],
    len: u8,
}

impl ObjectPath {
    fn new(prefix: &str, content: ContentId) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = [0; 96];
        bytes[..prefix.len()].copy_from_slice(prefix.as_bytes());
        let mut cursor = prefix.len();
        for byte in content.0 {
            bytes[cursor] = HEX[(byte >> 4) as usize];
            bytes[cursor + 1] = HEX[(byte & 0x0f) as usize];
            cursor += 2
        }
        Self {
            bytes,
            len: cursor as u8,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("object path invariant")
    }
}

fn persist_object<const BLOCKS: usize>(
    fs: &mut SynFs<BLOCKS>,
    path: &str,
    contents: &[u8],
    verification_buffer: &mut [u8],
) -> Result<(), RepositoryError> {
    match fs.lookup(path) {
        Ok(file) => {
            if file.size != contents.len() as u64 {
                return Err(RepositoryError::CorruptObject);
            }
            if verification_buffer.len() < contents.len() {
                return Err(RepositoryError::VerificationBufferTooSmall {
                    required: contents.len(),
                });
            }
            let read = fs
                .read(path, verification_buffer)
                .map_err(RepositoryError::SynFs)?;
            if read.bytes_read != contents.len()
                || verification_buffer[..read.bytes_read] != *contents
            {
                return Err(RepositoryError::CorruptObject);
            }
        }
        Err(SynFsError::NotFound) => {
            fs.write(path, contents).map_err(RepositoryError::SynFs)?;
        }
        Err(error) => return Err(RepositoryError::SynFs(error)),
    }
    Ok(())
}

fn encode_package_identity(
    payload: ContentId,
    byte_length: u64,
    entry_offset: u64,
    dependencies: &[Option<ContentId>; MAX_DEPENDENCIES],
) -> ([u8; PACKAGE_IDENTITY_BYTES], usize) {
    let mut identity = [0u8; PACKAGE_IDENTITY_BYTES];
    identity[..8].copy_from_slice(b"SYNPKG01");
    identity[8..40].copy_from_slice(payload.as_bytes());
    identity[40..48].copy_from_slice(&byte_length.to_be_bytes());
    identity[48..56].copy_from_slice(&entry_offset.to_be_bytes());
    identity[56] = dependencies.iter().flatten().count() as u8;
    let mut identity_length = 57;
    for dependency in dependencies.iter().flatten() {
        identity[identity_length..identity_length + 32].copy_from_slice(dependency.as_bytes());
        identity_length += 32
    }
    (identity, identity_length)
}

fn encode_root_snapshot(
    manifest: &RootManifest<DEFAULT_ROOT_BINDINGS>,
) -> ([u8; ROOT_SNAPSHOT_BYTES], usize) {
    let mut snapshot = [0u8; ROOT_SNAPSHOT_BYTES];
    snapshot[..8].copy_from_slice(b"SYNROOT1");
    snapshot[8..16].copy_from_slice(&manifest.revision().to_be_bytes());
    snapshot[16..18].copy_from_slice(&(manifest.bindings().count() as u16).to_be_bytes());
    let mut cursor = 18;
    for binding in manifest.bindings() {
        let name = binding.name.as_str().as_bytes();
        snapshot[cursor] = name.len() as u8;
        cursor += 1;
        snapshot[cursor..cursor + name.len()].copy_from_slice(name);
        cursor += MAX_NAME_BYTES;
        snapshot[cursor..cursor + 32].copy_from_slice(binding.package.as_bytes());
        cursor += 32
    }
    (snapshot, cursor)
}

fn sha256(message: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut state = INITIAL;
    let full_blocks = message.len() / 64;
    for block in 0..full_blocks {
        compress(&mut state, &message[block * 64..block * 64 + 64], &ROUND)
    }

    let remainder = &message[full_blocks * 64..];
    let mut tail = [0u8; 128];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;
    let tail_length = if remainder.len() < 56 { 64 } else { 128 };
    let bit_length = (message.len() as u64).wrapping_mul(8).to_be_bytes();
    tail[tail_length - 8..tail_length].copy_from_slice(&bit_length);
    compress(&mut state, &tail[..64], &ROUND);
    if tail_length == 128 {
        compress(&mut state, &tail[64..], &ROUND)
    }

    let mut digest = [0u8; 32];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes())
    }
    digest
}

fn compress(state: &mut [u32; 8], block: &[u8], round: &[u32; 64]) {
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
            .wrapping_add(round[index])
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
