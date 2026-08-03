#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Status};
use synos_synfs::SynFs;
use synos_system_model::{
    ContentId, DEFAULT_PACKAGE_CAPACITY, DEFAULT_ROOT_BINDINGS, Error as ModelError, LogicalName,
    MAX_DEPENDENCIES, PackageManifest, RepositoryError, RootBuilder, RootManifest, SynFsRepository,
};

const BUNDLE_MAGIC: &[u8; 8] = b"SYNBNDL1";
const BUNDLE_VERSION: u16 = 1;
const FIXED_HEADER_BYTES: usize = 144;
const MAX_SIGNED_BYTES: usize = 107 + MAX_DEPENDENCIES * 32;
pub const KEY_ID_BYTES: usize = 16;
pub const SIGNATURE_BYTES: usize = 32;
pub const DEFAULT_TRUSTED_KEYS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageError {
    BundleTooSmall,
    BufferTooSmall { required: usize },
    CorruptBundle,
    DuplicateBinding,
    DuplicateKey,
    InvalidConfiguration,
    InvalidSignature,
    Model(ModelError),
    Repository(RepositoryError),
    TooManyDependencies,
    TrustStoreFull,
    UnknownSigningKey,
}

impl IntoStatus for PackageError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } | Self::TooManyDependencies | Self::TrustStoreFull => {
                Status::NO_SPACE
            }
            Self::UnknownSigningKey | Self::InvalidSignature => Status::ACCESS_DENIED,
            Self::CorruptBundle | Self::BundleTooSmall => Status::CORRUPT,
            Self::InvalidConfiguration | Self::DuplicateBinding | Self::DuplicateKey => {
                Status::INVALID_ARGUMENT
            }
            Self::Model(error) => error.status(),
            Self::Repository(error) => error.status(),
        }
    }
}

impl From<ModelError> for PackageError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl From<RepositoryError> for PackageError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

/// Symmetric package-signing key used by the initial offline trust model.
///
/// Production key material is expected to be supplied by a capability-guarded
/// keystore. Bundles contain only the derived key identifier.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SigningKey([u8; 32]);

impl SigningKey {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn id(self) -> [u8; KEY_ID_BYTES] {
        let digest = ContentId::hash(&self.0);
        let mut id = [0; KEY_ID_BYTES];
        id.copy_from_slice(&digest.as_bytes()[..KEY_ID_BYTES]);
        id
    }

    fn sign(self, message: &[u8]) -> [u8; SIGNATURE_BYTES] {
        hmac_sha256(&self.0, message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundleInfo {
    pub package: ContentId,
    pub payload: ContentId,
    pub payload_length: u64,
    pub entry_offset: u64,
    pub signing_key: [u8; KEY_ID_BYTES],
    pub dependency_count: u8,
}

pub struct PackageBundle<'a> {
    info: BundleInfo,
    dependencies: [Option<ContentId>; MAX_DEPENDENCIES],
    signature: [u8; SIGNATURE_BYTES],
    payload: &'a [u8],
}

impl<'a> PackageBundle<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, PackageError> {
        if bytes.len() < FIXED_HEADER_BYTES {
            return Err(PackageError::BundleTooSmall);
        }
        if &bytes[..8] != BUNDLE_MAGIC || read_u16(bytes, 8)? != BUNDLE_VERSION {
            return Err(PackageError::CorruptBundle);
        }

        let dependency_count = bytes[28] as usize;
        if dependency_count > MAX_DEPENDENCIES {
            return Err(PackageError::TooManyDependencies);
        }
        if bytes[29..32].iter().any(|byte| *byte != 0) {
            return Err(PackageError::CorruptBundle);
        }

        let header_length = read_u16(bytes, 10)? as usize;
        let expected_header = FIXED_HEADER_BYTES + dependency_count * 32;
        if header_length != expected_header {
            return Err(PackageError::CorruptBundle);
        }
        let payload_length = read_u64(bytes, 12)?;
        let payload_size =
            usize::try_from(payload_length).map_err(|_| PackageError::CorruptBundle)?;
        let expected_length = header_length
            .checked_add(payload_size)
            .ok_or(PackageError::CorruptBundle)?;
        if bytes.len() != expected_length {
            return Err(PackageError::CorruptBundle);
        }

        let entry_offset = read_u64(bytes, 20)?;
        if entry_offset > payload_length {
            return Err(PackageError::CorruptBundle);
        }

        let package = read_content_id(bytes, 32)?;
        let payload_id = read_content_id(bytes, 64)?;
        let mut signing_key = [0; KEY_ID_BYTES];
        signing_key.copy_from_slice(&bytes[96..112]);
        let mut signature = [0; SIGNATURE_BYTES];
        signature.copy_from_slice(&bytes[112..144]);

        let mut dependencies = [None; MAX_DEPENDENCIES];
        for (index, slot) in dependencies.iter_mut().take(dependency_count).enumerate() {
            *slot = Some(read_content_id(bytes, FIXED_HEADER_BYTES + index * 32)?)
        }
        let payload = &bytes[header_length..];
        let dependency_ids = dependencies
            .iter()
            .flatten()
            .copied()
            .collect::<DependencyArray>();
        let manifest = PackageManifest::new(payload, entry_offset, dependency_ids.as_slice())?;
        if manifest.content != package || manifest.payload != payload_id {
            return Err(PackageError::CorruptBundle);
        }

        Ok(Self {
            info: BundleInfo {
                package,
                payload: payload_id,
                payload_length,
                entry_offset,
                signing_key,
                dependency_count: dependency_count as u8,
            },
            dependencies,
            signature,
            payload,
        })
    }

    pub const fn info(&self) -> BundleInfo {
        self.info
    }

    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn dependencies(&self) -> impl Iterator<Item = ContentId> + '_ {
        self.dependencies.iter().flatten().copied()
    }

    pub fn verify(&self, key: SigningKey) -> Result<(), PackageError> {
        if key.id() != self.info.signing_key {
            return Err(PackageError::UnknownSigningKey);
        }
        let (material, length) = signature_material(
            self.info.package,
            self.info.payload,
            self.info.payload_length,
            self.info.entry_offset,
            self.info.signing_key,
            &self.dependencies,
        );
        let expected = key.sign(&material[..length]);
        if constant_time_equal(&expected, &self.signature) {
            Ok(())
        } else {
            Err(PackageError::InvalidSignature)
        }
    }
}

pub fn bundle_size(payload_length: usize, dependency_count: usize) -> Result<usize, PackageError> {
    if dependency_count > MAX_DEPENDENCIES {
        return Err(PackageError::TooManyDependencies);
    }
    FIXED_HEADER_BYTES
        .checked_add(dependency_count * 32)
        .and_then(|length| length.checked_add(payload_length))
        .ok_or(PackageError::BufferTooSmall {
            required: usize::MAX,
        })
}

pub fn encode_bundle(
    payload: &[u8],
    entry_offset: u64,
    dependencies: &[ContentId],
    key: SigningKey,
    destination: &mut [u8],
) -> Result<BundleInfo, PackageError> {
    let required = bundle_size(payload.len(), dependencies.len())?;
    if destination.len() < required {
        return Err(PackageError::BufferTooSmall { required });
    }
    let manifest = PackageManifest::new(payload, entry_offset, dependencies)?;
    let header_length = FIXED_HEADER_BYTES + dependencies.len() * 32;
    let key_id = key.id();

    destination[..required].fill(0);
    destination[..8].copy_from_slice(BUNDLE_MAGIC);
    destination[8..10].copy_from_slice(&BUNDLE_VERSION.to_be_bytes());
    destination[10..12].copy_from_slice(&(header_length as u16).to_be_bytes());
    destination[12..20].copy_from_slice(&(payload.len() as u64).to_be_bytes());
    destination[20..28].copy_from_slice(&entry_offset.to_be_bytes());
    destination[28] = dependencies.len() as u8;
    destination[32..64].copy_from_slice(manifest.content.as_bytes());
    destination[64..96].copy_from_slice(manifest.payload.as_bytes());
    destination[96..112].copy_from_slice(&key_id);
    for (index, dependency) in dependencies.iter().enumerate() {
        let offset = FIXED_HEADER_BYTES + index * 32;
        destination[offset..offset + 32].copy_from_slice(dependency.as_bytes())
    }
    destination[header_length..required].copy_from_slice(payload);

    let stored_dependencies = dependencies_to_array(dependencies)?;
    let (material, material_length) = signature_material(
        manifest.content,
        manifest.payload,
        payload.len() as u64,
        entry_offset,
        key_id,
        &stored_dependencies,
    );
    destination[112..144].copy_from_slice(&key.sign(&material[..material_length]));

    Ok(BundleInfo {
        package: manifest.content,
        payload: manifest.payload,
        payload_length: payload.len() as u64,
        entry_offset,
        signing_key: key_id,
        dependency_count: dependencies.len() as u8,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationBinding {
    pub name: LogicalName,
    pub package: ContentId,
}

/// Bounded declarative description of the complete active package root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemConfiguration {
    revision: u64,
    bindings: [Option<ConfigurationBinding>; DEFAULT_ROOT_BINDINGS],
}

impl SystemConfiguration {
    pub const fn new(revision: u64) -> Self {
        Self {
            revision,
            bindings: [None; DEFAULT_ROOT_BINDINGS],
        }
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn bindings(&self) -> impl Iterator<Item = ConfigurationBinding> + '_ {
        self.bindings.iter().flatten().copied()
    }

    pub fn bind(&mut self, name: &str, package: ContentId) -> Result<(), PackageError> {
        let name = LogicalName::new(name)?;
        if self.bindings().any(|binding| binding.name == name) {
            return Err(PackageError::DuplicateBinding);
        }
        let slot = self
            .bindings
            .iter_mut()
            .find(|binding| binding.is_none())
            .ok_or(PackageError::InvalidConfiguration)?;
        *slot = Some(ConfigurationBinding { name, package });
        Ok(())
    }

    fn root(&self) -> Result<RootManifest<DEFAULT_ROOT_BINDINGS>, PackageError> {
        if self.revision == 0 {
            return Err(PackageError::InvalidConfiguration);
        }
        let mut root = RootBuilder::new(self.revision);
        for binding in self.bindings() {
            root.bind(binding.name, binding.package)?
        }
        Ok(root.seal()?)
    }
}

pub struct PackageDaemon<
    const PACKAGES: usize = DEFAULT_PACKAGE_CAPACITY,
    const KEYS: usize = DEFAULT_TRUSTED_KEYS,
> {
    repository: SynFsRepository<PACKAGES>,
    trusted_keys: [Option<SigningKey>; KEYS],
}

impl<const PACKAGES: usize, const KEYS: usize> PackageDaemon<PACKAGES, KEYS> {
    pub const fn new() -> Self {
        Self {
            repository: SynFsRepository::new(),
            trusted_keys: [None; KEYS],
        }
    }

    pub fn trust_key(&mut self, key: SigningKey) -> Result<[u8; KEY_ID_BYTES], PackageError> {
        let key_id = key.id();
        if self
            .trusted_keys
            .iter()
            .flatten()
            .any(|trusted| trusted.id() == key_id)
        {
            return Err(PackageError::DuplicateKey);
        }
        let slot = self
            .trusted_keys
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PackageError::TrustStoreFull)?;
        *slot = Some(key);
        Ok(key_id)
    }

    pub fn install_bundle<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        encoded: &[u8],
        verification_buffer: &mut [u8],
    ) -> Result<ContentId, PackageError> {
        let bundle = PackageBundle::decode(encoded)?;
        let key = self
            .trusted_keys
            .iter()
            .flatten()
            .find(|key| key.id() == bundle.info.signing_key)
            .copied()
            .ok_or(PackageError::UnknownSigningKey)?;
        bundle.verify(key)?;
        let dependencies = bundle.dependencies().collect::<DependencyArray>();
        let installed = self.repository.install(
            fs,
            bundle.payload(),
            bundle.info.entry_offset,
            dependencies.as_slice(),
            verification_buffer,
        )?;
        if installed != bundle.info.package {
            return Err(PackageError::CorruptBundle);
        }
        Ok(installed)
    }

    pub fn activate<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        configuration: &SystemConfiguration,
    ) -> Result<Option<RootManifest<DEFAULT_ROOT_BINDINGS>>, PackageError> {
        Ok(self.repository.activate(fs, configuration.root()?)?)
    }

    pub const fn active_configuration(&self) -> Option<&RootManifest<DEFAULT_ROOT_BINDINGS>> {
        self.repository.active_root()
    }

    pub fn restore_configuration(
        &mut self,
        configuration: Option<RootManifest<DEFAULT_ROOT_BINDINGS>>,
    ) -> Result<(), PackageError> {
        self.repository.restore_root(configuration)?;
        Ok(())
    }

    pub fn contains(&self, package: ContentId) -> bool {
        self.repository.packages().contains(package)
    }

    pub fn manifest(&self, package: ContentId) -> Option<&PackageManifest> {
        self.repository.packages().get(package)
    }
}

impl<const PACKAGES: usize, const KEYS: usize> Default for PackageDaemon<PACKAGES, KEYS> {
    fn default() -> Self {
        Self::new()
    }
}

struct DependencyArray {
    values: [ContentId; MAX_DEPENDENCIES],
    length: usize,
}

impl DependencyArray {
    fn as_slice(&self) -> &[ContentId] {
        &self.values[..self.length]
    }
}

impl FromIterator<ContentId> for DependencyArray {
    fn from_iter<T: IntoIterator<Item = ContentId>>(iter: T) -> Self {
        let mut values = [ContentId::from_bytes([0; 32]); MAX_DEPENDENCIES];
        let mut length = 0;
        for value in iter.into_iter().take(MAX_DEPENDENCIES) {
            values[length] = value;
            length += 1
        }
        Self { values, length }
    }
}

fn dependencies_to_array(
    dependencies: &[ContentId],
) -> Result<[Option<ContentId>; MAX_DEPENDENCIES], PackageError> {
    if dependencies.len() > MAX_DEPENDENCIES {
        return Err(PackageError::TooManyDependencies);
    }
    let mut stored = [None; MAX_DEPENDENCIES];
    for (slot, dependency) in stored.iter_mut().zip(dependencies) {
        *slot = Some(*dependency)
    }
    Ok(stored)
}

fn signature_material(
    package: ContentId,
    payload: ContentId,
    payload_length: u64,
    entry_offset: u64,
    key_id: [u8; KEY_ID_BYTES],
    dependencies: &[Option<ContentId>; MAX_DEPENDENCIES],
) -> ([u8; MAX_SIGNED_BYTES], usize) {
    let mut material = [0; MAX_SIGNED_BYTES];
    material[..8].copy_from_slice(BUNDLE_MAGIC);
    material[8..10].copy_from_slice(&BUNDLE_VERSION.to_be_bytes());
    material[10..18].copy_from_slice(&payload_length.to_be_bytes());
    material[18..26].copy_from_slice(&entry_offset.to_be_bytes());
    material[26..58].copy_from_slice(package.as_bytes());
    material[58..90].copy_from_slice(payload.as_bytes());
    material[90..106].copy_from_slice(&key_id);
    material[106] = dependencies.iter().flatten().count() as u8;
    let mut cursor = 107;
    for dependency in dependencies.iter().flatten() {
        material[cursor..cursor + 32].copy_from_slice(dependency.as_bytes());
        cursor += 32
    }
    (material, cursor)
}

fn read_content_id(bytes: &[u8], offset: usize) -> Result<ContentId, PackageError> {
    let end = offset.checked_add(32).ok_or(PackageError::CorruptBundle)?;
    let source = bytes.get(offset..end).ok_or(PackageError::CorruptBundle)?;
    let mut content = [0; 32];
    content.copy_from_slice(source);
    Ok(ContentId::from_bytes(content))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PackageError> {
    let source = bytes
        .get(offset..offset + 2)
        .ok_or(PackageError::CorruptBundle)?;
    Ok(u16::from_be_bytes([source[0], source[1]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, PackageError> {
    let source = bytes
        .get(offset..offset + 8)
        .ok_or(PackageError::CorruptBundle)?;
    Ok(u64::from_be_bytes([
        source[0], source[1], source[2], source[3], source[4], source[5], source[6], source[7],
    ]))
}

fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0; 64];
    normalized[..key.len()].copy_from_slice(key);

    let mut inner = [0; 64 + MAX_SIGNED_BYTES];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36
    }
    inner[64..64 + message.len()].copy_from_slice(message);
    let inner_hash = ContentId::hash(&inner[..64 + message.len()]);

    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c
    }
    outer[64..].copy_from_slice(inner_hash.as_bytes());
    *ContentId::hash(&outer).as_bytes()
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
