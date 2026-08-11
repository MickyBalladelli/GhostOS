#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Status};
use synos_durability::{InterruptionInjector, NoInterruption};
use synos_synfs::{
    CapacityObservation, CapacityResource, SynFs,
};
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
pub const APPLICATION_BUNDLE_MAGIC: &[u8; 8] = b"SYNAPP01";
pub const APPLICATION_BUNDLE_VERSION: u16 = 1;
pub const APPLICATION_BUNDLE_HEADER_BYTES: usize = 144;
pub const APPLICATION_METADATA_BYTES: usize = 160;
pub const PROVENANCE_LINKS: usize = 7;
pub const PROVENANCE_LINK_BYTES: usize = 113;
pub const PROVENANCE_HEADER_BYTES: usize = 12;
pub const PROVENANCE_CHAIN_BYTES: usize =
    PROVENANCE_HEADER_BYTES + PROVENANCE_LINKS * PROVENANCE_LINK_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageError {
    BundleTooSmall,
    BufferTooSmall { required: usize },
    CorruptBundle,
    DuplicateBinding,
    DuplicateKey,
    InvalidConfiguration,
    InvalidSignature,
    InstantiationDenied,
    Model(ModelError),
    Repository(RepositoryError),
    Interrupted,
    TooManyDependencies,
    TrustStoreFull,
    UnknownSigningKey,
    Provenance(ProvenanceError),
}

impl IntoStatus for PackageError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } | Self::TooManyDependencies | Self::TrustStoreFull => {
                Status::NO_SPACE
            }
            Self::UnknownSigningKey | Self::InvalidSignature | Self::InstantiationDenied => {
                Status::ACCESS_DENIED
            }
            Self::CorruptBundle | Self::BundleTooSmall => Status::CORRUPT,
            Self::Provenance(error) => error.status(),
            Self::InvalidConfiguration | Self::DuplicateBinding | Self::DuplicateKey => {
                Status::INVALID_ARGUMENT
            }
            Self::Model(error) => error.status(),
            Self::Repository(error) => error.status(),
            Self::Interrupted => Status::BUSY,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvenanceError {
    Capacity,
    Corrupt,
    InvalidSignature,
    InvalidStage,
    MissingStage,
}

impl ProvenanceError {
    const fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::Corrupt | Self::InvalidSignature => Status::CORRUPT,
            Self::InvalidStage | Self::MissingStage => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProvenanceStage {
    SourceSnapshot = 1,
    Toolchain = 2,
    Dependencies = 3,
    CompilerResult = 4,
    Package = 5,
    Activation = 6,
    RunningProcess = 7,
}

impl ProvenanceStage {
    const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::SourceSnapshot,
            1 => Self::Toolchain,
            2 => Self::Dependencies,
            3 => Self::CompilerResult,
            4 => Self::Package,
            5 => Self::Activation,
            _ => Self::RunningProcess,
        }
    }

    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::SourceSnapshot),
            2 => Some(Self::Toolchain),
            3 => Some(Self::Dependencies),
            4 => Some(Self::CompilerResult),
            5 => Some(Self::Package),
            6 => Some(Self::Activation),
            7 => Some(Self::RunningProcess),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceLink {
    pub stage: ProvenanceStage,
    pub subject: ContentId,
    pub previous: ContentId,
    pub signing_key: [u8; KEY_ID_BYTES],
    pub signature: [u8; SIGNATURE_BYTES],
}

impl ProvenanceLink {
    fn material(self) -> [u8; 81] {
        let mut material = [0; 81];
        material[0] = self.stage as u8;
        material[1..33].copy_from_slice(self.subject.as_bytes());
        material[33..65].copy_from_slice(self.previous.as_bytes());
        material[65..81].copy_from_slice(&self.signing_key);
        material
    }

    pub fn content_id(self) -> ContentId {
        let mut bytes = [0; PROVENANCE_LINK_BYTES];
        self.encode(&mut bytes);
        ContentId::hash(&bytes)
    }

    fn encode(self, destination: &mut [u8]) {
        destination[0] = self.stage as u8;
        destination[1..33].copy_from_slice(self.subject.as_bytes());
        destination[33..65].copy_from_slice(self.previous.as_bytes());
        destination[65..81].copy_from_slice(&self.signing_key);
        destination[81..113].copy_from_slice(&self.signature);
    }

    fn decode(bytes: &[u8]) -> Result<Self, ProvenanceError> {
        if bytes.len() != PROVENANCE_LINK_BYTES {
            return Err(ProvenanceError::Corrupt)
        }
        let stage = ProvenanceStage::from_raw(bytes[0]).ok_or(ProvenanceError::Corrupt)?;
        let mut subject = [0; 32];
        subject.copy_from_slice(&bytes[1..33]);
        let mut previous = [0; 32];
        previous.copy_from_slice(&bytes[33..65]);
        let mut signing_key = [0; KEY_ID_BYTES];
        signing_key.copy_from_slice(&bytes[65..81]);
        let mut signature = [0; SIGNATURE_BYTES];
        signature.copy_from_slice(&bytes[81..113]);
        Ok(Self {
            stage,
            subject: ContentId::from_bytes(subject),
            previous: ContentId::from_bytes(previous),
            signing_key,
            signature,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceChain {
    links: [Option<ProvenanceLink>; PROVENANCE_LINKS],
    length: u8,
}

impl ProvenanceChain {
    pub fn new(source: ContentId, key: SigningKey) -> Result<Self, PackageError> {
        let mut chain = Self {
            links: [None; PROVENANCE_LINKS],
            length: 0,
        };
        chain.append(ProvenanceStage::SourceSnapshot, source, key)?;
        Ok(chain)
    }

    pub fn append(
        &mut self,
        stage: ProvenanceStage,
        subject: ContentId,
        key: SigningKey,
    ) -> Result<(), PackageError> {
        let index = self.length as usize;
        if index >= PROVENANCE_LINKS {
            return Err(PackageError::Provenance(ProvenanceError::Capacity))
        }
        if subject.is_zero() || stage != ProvenanceStage::from_index(index) {
            return Err(PackageError::Provenance(ProvenanceError::InvalidStage))
        }
        let signing_key = key.id();
        if index != 0 && self.links[0].is_some_and(|link| link.signing_key != signing_key) {
            return Err(PackageError::Provenance(ProvenanceError::InvalidSignature))
        }
        let previous = self
            .links
            .iter()
            .flatten()
            .last()
            .map(|link| link.content_id())
            .unwrap_or_else(|| ContentId::from_bytes([0; 32]));
        let unsigned = ProvenanceLink {
            stage,
            subject,
            previous,
            signing_key,
            signature: [0; SIGNATURE_BYTES],
        };
        let mut link = unsigned;
        link.signature = key.sign(&unsigned.material());
        self.links[index] = Some(link);
        self.length += 1;
        Ok(())
    }

    pub fn verify(&self, key: SigningKey) -> Result<(), PackageError> {
        let signing_key = key.id();
        let mut previous = ContentId::from_bytes([0; 32]);
        for (index, link) in self.links.iter().take(self.length as usize).flatten().enumerate() {
            if link.stage != ProvenanceStage::from_index(index)
                || link.signing_key != signing_key
                || link.previous != previous
                || link.subject.is_zero()
                || !constant_time_equal(&key.sign(&link.material()), &link.signature)
            {
                return Err(PackageError::Provenance(ProvenanceError::InvalidSignature))
            }
            previous = link.content_id();
        }
        if self.length == 0 {
            return Err(PackageError::Provenance(ProvenanceError::Corrupt))
        }
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.length as usize
    }

    pub fn links(&self) -> impl Iterator<Item = ProvenanceLink> + '_ {
        self.links.iter().take(self.length as usize).flatten().copied()
    }

    pub fn subject(&self, stage: ProvenanceStage) -> Option<ContentId> {
        self.links()
            .find(|link| link.stage == stage)
            .map(|link| link.subject)
    }

    pub fn signing_key(&self) -> Option<[u8; KEY_ID_BYTES]> {
        self.links[0].map(|link| link.signing_key)
    }

    pub fn content_id(self) -> ContentId {
        let mut encoded = [0; PROVENANCE_CHAIN_BYTES];
        self.encode(&mut encoded).expect("fixed provenance buffer");
        ContentId::hash(&encoded)
    }

    pub fn encode(&self, destination: &mut [u8]) -> Result<(), PackageError> {
        if destination.len() < PROVENANCE_CHAIN_BYTES {
            return Err(PackageError::BufferTooSmall {
                required: PROVENANCE_CHAIN_BYTES,
            })
        }
        destination[..PROVENANCE_CHAIN_BYTES].fill(0);
        destination[..8].copy_from_slice(b"SYNPROV1");
        destination[8..10].copy_from_slice(&1_u16.to_be_bytes());
        destination[10] = self.length;
        for (index, link) in self.links().enumerate() {
            let offset = PROVENANCE_HEADER_BYTES + index * PROVENANCE_LINK_BYTES;
            link.encode(&mut destination[offset..offset + PROVENANCE_LINK_BYTES]);
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() != PROVENANCE_CHAIN_BYTES
            || &bytes[..8] != b"SYNPROV1"
            || u16::from_be_bytes([bytes[8], bytes[9]]) != 1
            || bytes[10] as usize > PROVENANCE_LINKS
            || bytes[11] != 0
        {
            return Err(PackageError::Provenance(ProvenanceError::Corrupt))
        }
        let length = bytes[10] as usize;
        let mut links = [None; PROVENANCE_LINKS];
        for (index, slot) in links.iter_mut().take(length).enumerate() {
            let offset = PROVENANCE_HEADER_BYTES + index * PROVENANCE_LINK_BYTES;
            *slot = Some(ProvenanceLink::decode(
                &bytes[offset..offset + PROVENANCE_LINK_BYTES],
            ).map_err(PackageError::Provenance)?);
        }
        if bytes[PROVENANCE_HEADER_BYTES + length * PROVENANCE_LINK_BYTES..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(PackageError::Provenance(ProvenanceError::Corrupt))
        }
        Ok(Self {
            links,
            length: length as u8,
        })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationPackageManifest {
    pub schema: u16,
    pub target: u8,
    pub kind: u8,
    pub name: [u8; 48],
    pub name_length: u8,
    pub entry_offset: u64,
    pub memory_bytes: u64,
    pub cpu_time_us: u64,
    pub heap_bytes: u64,
    pub debug_symbols: ContentId,
    pub build_record: ContentId,
}

impl ApplicationPackageManifest {
    pub fn new(
        schema: u16,
        target: u8,
        kind: u8,
        name: &str,
        entry_offset: u64,
        memory_bytes: u64,
        cpu_time_us: u64,
        heap_bytes: u64,
        debug_symbols: ContentId,
        build_record: ContentId,
    ) -> Result<Self, PackageError> {
        if name.is_empty() || name.len() > 48 || memory_bytes == 0 || heap_bytes == 0 {
            return Err(PackageError::InvalidConfiguration);
        }
        let mut stored_name = [0; 48];
        stored_name[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            schema,
            target,
            kind,
            name: stored_name,
            name_length: name.len() as u8,
            entry_offset,
            memory_bytes,
            cpu_time_us,
            heap_bytes,
            debug_symbols,
            build_record,
        })
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_length as usize]).unwrap_or("")
    }

    fn encode(self, destination: &mut [u8]) -> Result<(), PackageError> {
        if destination.len() < APPLICATION_METADATA_BYTES {
            return Err(PackageError::BufferTooSmall {
                required: APPLICATION_METADATA_BYTES,
            });
        }
        destination[..APPLICATION_METADATA_BYTES].fill(0);
        destination[0..2].copy_from_slice(&self.schema.to_be_bytes());
        destination[2] = self.target;
        destination[3] = self.kind;
        destination[4] = self.name_length;
        destination[6..14].copy_from_slice(&self.entry_offset.to_be_bytes());
        destination[14..22].copy_from_slice(&self.memory_bytes.to_be_bytes());
        destination[22..30].copy_from_slice(&self.cpu_time_us.to_be_bytes());
        destination[30..38].copy_from_slice(&self.heap_bytes.to_be_bytes());
        destination[38..70].copy_from_slice(self.debug_symbols.as_bytes());
        destination[70..102].copy_from_slice(self.build_record.as_bytes());
        destination[102..150].copy_from_slice(&self.name);
        Ok(())
    }

    fn decode(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() != APPLICATION_METADATA_BYTES {
            return Err(PackageError::CorruptBundle);
        }
        let name_length = bytes[4] as usize;
        if name_length == 0 || name_length > 48 || bytes[150..].iter().any(|byte| *byte != 0) {
            return Err(PackageError::CorruptBundle);
        }
        let mut name = [0; 48];
        name.copy_from_slice(&bytes[102..150]);
        core::str::from_utf8(&name[..name_length]).map_err(|_| PackageError::CorruptBundle)?;
        Ok(Self {
            schema: read_u16(bytes, 0)?,
            target: bytes[2],
            kind: bytes[3],
            name,
            name_length: name_length as u8,
            entry_offset: read_u64(bytes, 6)?,
            memory_bytes: read_u64(bytes, 14)?,
            cpu_time_us: read_u64(bytes, 22)?,
            heap_bytes: read_u64(bytes, 30)?,
            debug_symbols: read_content_id(bytes, 38)?,
            build_record: read_content_id(bytes, 70)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationBundleInfo {
    pub package: ContentId,
    pub manifest: ContentId,
    pub signing_key: [u8; KEY_ID_BYTES],
    pub inner_length: u64,
}

pub struct ApplicationBundle<'a> {
    info: ApplicationBundleInfo,
    metadata: ApplicationPackageManifest,
    inner: &'a [u8],
    signature: [u8; SIGNATURE_BYTES],
}

impl<'a> ApplicationBundle<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, PackageError> {
        if bytes.len() < APPLICATION_BUNDLE_HEADER_BYTES + APPLICATION_METADATA_BYTES {
            return Err(PackageError::BundleTooSmall);
        }
        if &bytes[..8] != APPLICATION_BUNDLE_MAGIC
            || read_u16(bytes, 8)? != APPLICATION_BUNDLE_VERSION
            || read_u16(bytes, 10)? as usize != APPLICATION_BUNDLE_HEADER_BYTES
            || read_u64(bytes, 20)? as usize != APPLICATION_METADATA_BYTES
        {
            return Err(PackageError::CorruptBundle);
        }
        let inner_length = read_u64(bytes, 12)? as usize;
        let inner_end = APPLICATION_BUNDLE_HEADER_BYTES
            .checked_add(inner_length)
            .ok_or(PackageError::CorruptBundle)?;
        let metadata_end = inner_end
            .checked_add(APPLICATION_METADATA_BYTES)
            .ok_or(PackageError::CorruptBundle)?;
        if bytes.len() != metadata_end {
            return Err(PackageError::CorruptBundle);
        }
        let inner = &bytes[APPLICATION_BUNDLE_HEADER_BYTES..inner_end];
        let metadata = ApplicationPackageManifest::decode(&bytes[inner_end..])?;
        let inner_bundle = PackageBundle::decode(inner)?;
        if metadata.entry_offset != inner_bundle.info.entry_offset
            || bytes[140..APPLICATION_BUNDLE_HEADER_BYTES]
                .iter()
                .any(|byte| *byte != 0)
        {
            return Err(PackageError::CorruptBundle);
        }
        let mut manifest_material = [0; APPLICATION_METADATA_BYTES];
        metadata.encode(&mut manifest_material)?;
        let manifest_id = ContentId::hash(&manifest_material);
        if read_content_id(bytes, 28)? != inner_bundle.info.package
            || read_content_id(bytes, 60)? != manifest_id
        {
            return Err(PackageError::CorruptBundle);
        }
        let mut signing_key = [0; KEY_ID_BYTES];
        signing_key.copy_from_slice(&bytes[92..108]);
        let mut signature = [0; SIGNATURE_BYTES];
        signature.copy_from_slice(&bytes[108..140]);
        Ok(Self {
            info: ApplicationBundleInfo {
                package: inner_bundle.info.package,
                manifest: manifest_id,
                signing_key,
                inner_length: inner_length as u64,
            },
            metadata,
            inner,
            signature,
        })
    }

    pub const fn info(&self) -> ApplicationBundleInfo {
        self.info
    }

    pub const fn metadata(&self) -> ApplicationPackageManifest {
        self.metadata
    }

    pub const fn inner(&self) -> &'a [u8] {
        self.inner
    }

    pub fn verify(&self, key: SigningKey) -> Result<(), PackageError> {
        if key.id() != self.info.signing_key {
            return Err(PackageError::UnknownSigningKey);
        }
        let mut material = [0; 64];
        material[..32].copy_from_slice(ContentId::hash(self.inner).as_bytes());
        let mut metadata = [0; APPLICATION_METADATA_BYTES];
        self.metadata.encode(&mut metadata)?;
        material[32..].copy_from_slice(ContentId::hash(&metadata).as_bytes());
        if constant_time_equal(&key.sign(&material), &self.signature) {
            Ok(())
        } else {
            Err(PackageError::InvalidSignature)
        }
    }
}

pub fn application_bundle_size(inner_length: usize) -> Result<usize, PackageError> {
    APPLICATION_BUNDLE_HEADER_BYTES
        .checked_add(inner_length)
        .and_then(|length| length.checked_add(APPLICATION_METADATA_BYTES))
        .ok_or(PackageError::BufferTooSmall { required: usize::MAX })
}

pub fn encode_application_bundle(
    inner: &[u8],
    metadata: ApplicationPackageManifest,
    key: SigningKey,
    destination: &mut [u8],
) -> Result<ApplicationBundleInfo, PackageError> {
    let required = application_bundle_size(inner.len())?;
    if destination.len() < required {
        return Err(PackageError::BufferTooSmall { required });
    }
    let mut metadata_bytes = [0; APPLICATION_METADATA_BYTES];
    metadata.encode(&mut metadata_bytes)?;
    let inner_bundle = PackageBundle::decode(inner)?;
    if metadata.entry_offset != inner_bundle.info.entry_offset {
        return Err(PackageError::InvalidConfiguration);
    }
    let key_id = key.id();
    destination[..required].fill(0);
    destination[..8].copy_from_slice(APPLICATION_BUNDLE_MAGIC);
    destination[8..10].copy_from_slice(&APPLICATION_BUNDLE_VERSION.to_be_bytes());
    destination[10..12].copy_from_slice(&(APPLICATION_BUNDLE_HEADER_BYTES as u16).to_be_bytes());
    destination[12..20].copy_from_slice(&(inner.len() as u64).to_be_bytes());
    destination[20..28].copy_from_slice(&(APPLICATION_METADATA_BYTES as u64).to_be_bytes());
    destination[28..60].copy_from_slice(inner_bundle.info.package.as_bytes());
    destination[60..92].copy_from_slice(ContentId::hash(&metadata_bytes).as_bytes());
    destination[92..108].copy_from_slice(&key_id);
    destination[APPLICATION_BUNDLE_HEADER_BYTES..APPLICATION_BUNDLE_HEADER_BYTES + inner.len()]
        .copy_from_slice(inner);
    destination[APPLICATION_BUNDLE_HEADER_BYTES + inner.len()..required]
        .copy_from_slice(&metadata_bytes);
    let mut material = [0; 64];
    material[..32].copy_from_slice(ContentId::hash(inner).as_bytes());
    material[32..].copy_from_slice(ContentId::hash(&metadata_bytes).as_bytes());
    destination[108..140].copy_from_slice(&key.sign(&material));
    Ok(ApplicationBundleInfo {
        package: inner_bundle.info.package,
        manifest: ContentId::hash(&metadata_bytes),
        signing_key: key_id,
        inner_length: inner.len() as u64,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstantiationReceipt {
    package: ContentId,
    signing_key: [u8; KEY_ID_BYTES],
}

impl InstantiationReceipt {
    pub const fn package(self) -> ContentId {
        self.package
    }

    pub const fn signing_key(self) -> [u8; KEY_ID_BYTES] {
        self.signing_key
    }
}

#[derive(Clone, Copy)]
struct VerifiedPackage {
    package: ContentId,
    signing_key: [u8; KEY_ID_BYTES],
}

#[derive(Clone, Copy)]
struct InstalledApplication {
    package: ContentId,
    metadata: ApplicationPackageManifest,
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

    pub fn content_id(&self) -> ContentId {
        let mut material = [0; DEFAULT_ROOT_BINDINGS * (2 + 64 + 32) + 8];
        let mut cursor = 0;
        material[..8].copy_from_slice(&self.revision.to_be_bytes());
        cursor += 8;
        for binding in self.bindings() {
            let name = binding.name.as_str().as_bytes();
            material[cursor..cursor + 2].copy_from_slice(&(name.len() as u16).to_be_bytes());
            cursor += 2;
            material[cursor..cursor + name.len()].copy_from_slice(name);
            cursor += name.len();
            material[cursor..cursor + 32].copy_from_slice(binding.package.as_bytes());
            cursor += 32;
        }
        ContentId::hash(&material[..cursor])
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
    verified: [Option<VerifiedPackage>; PACKAGES],
    applications: [Option<InstalledApplication>; PACKAGES],
}

impl<const PACKAGES: usize, const KEYS: usize> PackageDaemon<PACKAGES, KEYS> {
    pub const fn new() -> Self {
        Self {
            repository: SynFsRepository::new(),
            trusted_keys: [None; KEYS],
            verified: [None; PACKAGES],
            applications: [None; PACKAGES],
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

    /// Remove a signing key and invalidate every receipt issued from it.
    /// Immutable package bytes remain available for forensic recovery, but
    /// they cannot cross the instantiation gate after revocation.
    pub fn revoke_key(&mut self, key_id: [u8; KEY_ID_BYTES]) -> Result<(), PackageError> {
        let slot = self
            .trusted_keys
            .iter_mut()
            .find(|entry| entry.is_some_and(|key| key.id() == key_id))
            .ok_or(PackageError::UnknownSigningKey)?;
        *slot = None;
        for entry in &mut self.verified {
            if entry.is_some_and(|verified| verified.signing_key == key_id) {
                *entry = None;
            }
        }
        Ok(())
    }

    pub fn install_bundle<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        encoded: &[u8],
        verification_buffer: &mut [u8],
    ) -> Result<ContentId, PackageError> {
        let bundle = PackageBundle::decode(encoded)?;
        let key = self.trusted_key(&bundle)?;
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
        self.record_verified(installed, bundle.info.signing_key)?;
        Ok(installed)
    }

    pub fn install_application_bundle<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        encoded: &[u8],
        verification_buffer: &mut [u8],
    ) -> Result<ApplicationBundleInfo, PackageError> {
        let application = ApplicationBundle::decode(encoded)?;
        let inner = PackageBundle::decode(application.inner())?;
        let key = self.trusted_key(&inner)?;
        inner.verify(key)?;
        application.verify(key)?;
        let dependencies = inner.dependencies().collect::<DependencyArray>();
        let installed = self.repository.install(
            fs,
            inner.payload(),
            inner.info.entry_offset,
            dependencies.as_slice(),
            verification_buffer,
        )?;
        if installed != application.info.package {
            return Err(PackageError::CorruptBundle);
        }
        self.record_verified(installed, inner.info.signing_key)?;
        self.record_application(installed, application.metadata())?;
        Ok(application.info())
    }

    /// Validate a bundle against the local TUF-style trusted key set without
    /// changing the repository.
    pub fn verify_bundle(&self, encoded: &[u8]) -> Result<BundleInfo, PackageError> {
        let bundle = PackageBundle::decode(encoded)?;
        let key = self.trusted_key(&bundle)?;
        bundle.verify(key)?;
        Ok(bundle.info())
    }

    pub fn activate<const BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        configuration: &SystemConfiguration,
    ) -> Result<Option<RootManifest<DEFAULT_ROOT_BINDINGS>>, PackageError> {
        let mut no_interruption = NoInterruption;
        self.activate_with_interruption(fs, configuration, &mut no_interruption)
    }

    pub fn activate_with_interruption<const BLOCKS: usize, I: InterruptionInjector>(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        configuration: &SystemConfiguration,
        injector: &mut I,
    ) -> Result<Option<RootManifest<DEFAULT_ROOT_BINDINGS>>, PackageError> {
        if configuration
            .bindings()
            .any(|binding| !self.is_instantiation_authorized(binding.package))
        {
            return Err(PackageError::InstantiationDenied);
        }
        Ok(self
            .repository
            .activate_with_interruption(fs, configuration.root()?, injector)
            .map_err(|error| match error {
                RepositoryError::Interrupted => PackageError::Interrupted,
                other => PackageError::Repository(other),
            })?)
    }

    /// Add the root activation to a verified build chain, then activate it.
    /// The chain is copied first, so an interrupted activation does not leave
    /// a provenance record for a root that never became active.
    pub fn activate_with_provenance<
        const BLOCKS: usize,
        I: InterruptionInjector,
    >(
        &mut self,
        fs: &mut SynFs<BLOCKS>,
        configuration: &SystemConfiguration,
        chain: &mut ProvenanceChain,
        injector: &mut I,
    ) -> Result<Option<RootManifest<DEFAULT_ROOT_BINDINGS>>, PackageError> {
        let mut next = *chain;
        self.append_provenance(&mut next, ProvenanceStage::Activation, configuration.content_id())?;
        let root = self.activate_with_interruption(fs, configuration, injector)?;
        *chain = next;
        Ok(root)
    }

    /// Append a trusted activation or running-process event to a chain.
    /// Only the signing key that verified the package may extend it.
    pub fn append_provenance(
        &self,
        chain: &mut ProvenanceChain,
        stage: ProvenanceStage,
        subject: ContentId,
    ) -> Result<(), PackageError> {
        if chain.subject(ProvenanceStage::Package)
            .is_none_or(|package| !self.is_instantiation_authorized(package))
        {
            return Err(PackageError::InstantiationDenied)
        }
        let signing_key = chain
            .signing_key()
            .ok_or(PackageError::Provenance(ProvenanceError::MissingStage))?;
        let key = self
            .trusted_keys
            .iter()
            .flatten()
            .find(|key| key.id() == signing_key)
            .copied()
            .ok_or(PackageError::UnknownSigningKey)?;
        let mut next = *chain;
        next.append(stage, subject, key)?;
        next.verify(key)?;
        *chain = next;
        Ok(())
    }

    pub fn verify_provenance(&self, chain: &ProvenanceChain) -> Result<(), PackageError> {
        let signing_key = chain
            .signing_key()
            .ok_or(PackageError::Provenance(ProvenanceError::MissingStage))?;
        let key = self
            .trusted_keys
            .iter()
            .flatten()
            .find(|key| key.id() == signing_key)
            .copied()
            .ok_or(PackageError::UnknownSigningKey)?;
        chain.verify(key)?;
        if chain.subject(ProvenanceStage::Package)
            .is_none_or(|package| !self.is_instantiation_authorized(package))
        {
            return Err(PackageError::InstantiationDenied)
        }
        Ok(())
    }

    pub fn record_running_process_provenance(
        &self,
        chain: &mut ProvenanceChain,
        process: u64,
        generation: u32,
    ) -> Result<(), PackageError> {
        let mut material = [0; 44];
        material[..32].copy_from_slice(
            chain
                .subject(ProvenanceStage::Package)
                .ok_or(PackageError::Provenance(ProvenanceError::MissingStage))?
                .as_bytes(),
        );
        material[32..40].copy_from_slice(&process.to_be_bytes());
        material[40..44].copy_from_slice(&generation.to_be_bytes());
        self.append_provenance(chain, ProvenanceStage::RunningProcess, ContentId::hash(&material))
    }

    pub const fn active_configuration(&self) -> Option<&RootManifest<DEFAULT_ROOT_BINDINGS>> {
        self.repository.active_root()
    }

    pub fn restore_configuration(
        &mut self,
        configuration: Option<RootManifest<DEFAULT_ROOT_BINDINGS>>,
    ) -> Result<(), PackageError> {
        if configuration.is_some_and(|root| {
            root.bindings()
                .any(|binding| !self.is_instantiation_authorized(binding.package))
        }) {
            return Err(PackageError::InstantiationDenied);
        }
        self.repository.restore_root(configuration)?;
        Ok(())
    }

    /// Return a receipt that the image passed the signed-package gate. A
    /// process launcher must hold this receipt before it instantiates code.
    pub fn authorize_instantiation(
        &self,
        package: ContentId,
    ) -> Result<InstantiationReceipt, PackageError> {
        self.verified
            .iter()
            .flatten()
            .find(|entry| entry.package == package && self.contains(package))
            .map(|entry| InstantiationReceipt {
                package: entry.package,
                signing_key: entry.signing_key,
            })
            .ok_or(PackageError::InstantiationDenied)
    }

    pub fn validate_instantiation(
        &self,
        receipt: InstantiationReceipt,
    ) -> Result<(), PackageError> {
        if self.is_instantiation_authorized(receipt.package)
            && self.verified.iter().flatten().any(|entry| {
                entry.package == receipt.package && entry.signing_key == receipt.signing_key
            })
        {
            Ok(())
        } else {
            Err(PackageError::InstantiationDenied)
        }
    }

    pub fn is_instantiation_authorized(&self, package: ContentId) -> bool {
        self.verified
            .iter()
            .flatten()
            .any(|entry| entry.package == package && self.contains(package))
    }

    pub fn contains(&self, package: ContentId) -> bool {
        self.repository.packages().contains(package)
    }

    pub fn manifest(&self, package: ContentId) -> Option<&PackageManifest> {
        self.repository.packages().get(package)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &PackageManifest> + '_ {
        self.repository.packages().iter()
    }

    pub fn capacity_observation<const BLOCKS: usize>(
        &self,
        filesystem: &SynFs<BLOCKS>,
        sampled_at_us: u64,
        growth_bytes_per_hour: u64,
    ) -> Result<CapacityObservation, PackageError> {
        let mut allocated_bytes = 0u64;
        for manifest in self.manifests() {
            allocated_bytes = allocated_bytes
                .saturating_add(manifest.byte_length)
                .saturating_add(313);
        }
        let fragmentation = filesystem
            .fragmentation_report()
            .map_err(|error| PackageError::Repository(RepositoryError::SynFs(error)))?;
        let block_bytes = synos_synfs::BLOCK_SIZE as u64;
        let fragmented_bytes = (fragmentation.fragmented_blocks as u64).saturating_mul(block_bytes);
        Ok(CapacityObservation {
            resource: CapacityResource::PackageCache,
            sampled_at_us,
            capacity_bytes: (filesystem.capacity() as u64).saturating_mul(block_bytes),
            allocated_bytes: allocated_bytes.max(fragmented_bytes),
            reclaimable_bytes: 0,
            fragmented_bytes,
            largest_free_extent_bytes: (fragmentation.largest_free_run as u64).saturating_mul(block_bytes),
            allocation_unit_bytes: block_bytes,
            gc_pending_bytes: 0,
            gc_work_limit_bytes: (BLOCKS as u64).saturating_mul(block_bytes),
            growth_bytes_per_hour,
        })
    }

    pub fn application_manifest(&self, package: ContentId) -> Option<ApplicationPackageManifest> {
        self.applications
            .iter()
            .flatten()
            .find(|entry| entry.package == package)
            .map(|entry| entry.metadata)
    }

    fn trusted_key(&self, bundle: &PackageBundle<'_>) -> Result<SigningKey, PackageError> {
        self.trusted_keys
            .iter()
            .flatten()
            .find(|key| key.id() == bundle.info.signing_key)
            .copied()
            .ok_or(PackageError::UnknownSigningKey)
    }

    fn record_verified(
        &mut self,
        package: ContentId,
        signing_key: [u8; KEY_ID_BYTES],
    ) -> Result<(), PackageError> {
        if let Some(entry) = self
            .verified
            .iter_mut()
            .flatten()
            .find(|entry| entry.package == package)
        {
            entry.signing_key = signing_key;
            return Ok(())
        }
        let slot = self
            .verified
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PackageError::InstantiationDenied)?;
        *slot = Some(VerifiedPackage {
            package,
            signing_key,
        });
        Ok(())
    }

    fn record_application(
        &mut self,
        package: ContentId,
        metadata: ApplicationPackageManifest,
    ) -> Result<(), PackageError> {
        if let Some(entry) = self
            .applications
            .iter_mut()
            .flatten()
            .find(|entry| entry.package == package)
        {
            entry.metadata = metadata;
            return Ok(())
        }
        let slot = self
            .applications
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(PackageError::InstantiationDenied)?;
        *slot = Some(InstalledApplication { package, metadata });
        Ok(())
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
