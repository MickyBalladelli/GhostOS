use crate::{
    capability::{CapabilityError, StorageCapability, StorageRights},
    cache::CacheMode,
    protocol::{Endpoint, EndpointError, Protocol, ProtocolFeatures},
};
use synos_observability::{
    field, BatchController, CapabilityDomain, CapabilityTrace, CapabilityTraceStage, EventField,
    EventKind, Level, ProducerPolicy,
};
use synos_numa::{NumaDecision, NumaPlacement, NumaReport, NumaTopology, PlacementKind};
use synos_ipc::{BufferCapability, BufferError, BufferLease, BufferOwner, BufferRights, SharedBuffer};

pub const MAX_MOUNTS: usize = 32;
pub const MAX_PENDING_IO: usize = 64;
pub const MAX_STORAGE_PATH_BYTES: usize = 256;
pub const ADMIN_MOUNT_ID: u32 = 1;
pub const MAX_STORAGE_BATCH: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoragePathError {
    InvalidPrefix,
    MissingMount,
    InvalidPath,
    TooLong,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct StoragePath {
    bytes: [u8; MAX_STORAGE_PATH_BYTES],
    len: u16,
}

impl StoragePath {
    pub const ROOT: Self = Self {
        bytes: {
            let mut bytes = [0; MAX_STORAGE_PATH_BYTES];
            bytes[0] = b'S';
            bytes[1] = b'Y';
            bytes[2] = b'S';
            bytes[3] = b'$';
            bytes[4] = b'S';
            bytes[5] = b'T';
            bytes[6] = b'O';
            bytes[7] = b'R';
            bytes[8] = b'A';
            bytes[9] = b'G';
            bytes[10] = b'E';
            bytes[11] = b':';
            bytes
        },
        len: 12,
    };

    pub fn new(value: &str) -> Result<Self, StoragePathError> {
        if value.len() > MAX_STORAGE_PATH_BYTES {
            return Err(StoragePathError::TooLong)
        }
        if !value.starts_with("SYS$STORAGE:") {
            return Err(StoragePathError::InvalidPrefix)
        }
        let suffix = &value[12..];
        if suffix.is_empty() {
            return Ok(Self::ROOT)
        }
        let Some(separator) = suffix.find('/') else {
            if suffix.contains(':') || suffix.contains('\0') {
                return Err(StoragePathError::InvalidPath)
            }
            let mut bytes = [0; MAX_STORAGE_PATH_BYTES];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            return Ok(Self {
                bytes,
                len: value.len() as u16,
            })
        };
        if separator == 0 || suffix.ends_with('/') || suffix.contains("//") {
            return Err(StoragePathError::InvalidPath)
        }
        if suffix
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\0'))
        {
            return Err(StoragePathError::InvalidPath)
        }
        let mut bytes = [0; MAX_STORAGE_PATH_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u16,
        })
    }

    pub fn mount_root(name: &str) -> Result<Self, StoragePathError> {
        let mut value = [0; MAX_STORAGE_PATH_BYTES];
        let prefix = b"SYS$STORAGE:";
        let length = prefix
            .len()
            .checked_add(name.len())
            .ok_or(StoragePathError::TooLong)?;
        if name.is_empty() || name.contains('/') || length > MAX_STORAGE_PATH_BYTES {
            return Err(StoragePathError::InvalidPath)
        }
        value[..prefix.len()].copy_from_slice(prefix);
        value[prefix.len()..length].copy_from_slice(name.as_bytes());
        let value = core::str::from_utf8(&value[..length]).map_err(|_| StoragePathError::InvalidPath)?;
        Self::new(value)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn contains(self, candidate: Self) -> bool {
        candidate == self
            || (self != Self::ROOT
                && candidate
                    .as_str()
                    .strip_prefix(self.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/')))
            || self == Self::ROOT && candidate.as_str().starts_with("SYS$STORAGE:")
    }

    pub fn mount_name(&self) -> &str {
        let suffix = &self.as_str()[12..];
        suffix.split_once('/').map_or(suffix, |(name, _)| name)
    }
}

impl core::fmt::Debug for StoragePath {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("StoragePath").field(&self.as_str()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountOptions {
    pub read_only: bool,
    pub cache: CacheMode,
    pub max_io_bytes: usize,
    pub retry_limit: u8,
}

impl MountOptions {
    pub const DEFAULT: Self = Self {
        read_only: false,
        cache: CacheMode::ReadThrough,
        max_io_bytes: 1024 * 1024,
        retry_limit: 3,
    };

    pub const fn validate(self) -> Result<Self, MountError> {
        if self.max_io_bytes == 0 || self.max_io_bytes > crate::MAX_IO_BYTES {
            Err(MountError::InvalidOptions)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountState {
    Mounted,
    Draining,
    Failed(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountId(u32);

impl MountId {
    pub const fn raw(self) -> u32 {
        self.0
    }

    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountInfo {
    pub id: MountId,
    pub logical: StoragePath,
    pub endpoint: Endpoint,
    pub protocol: Protocol,
    pub features: ProtocolFeatures,
    pub options: MountOptions,
    pub generation: u32,
    pub catalog_version: u64,
    pub state: MountState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum IoOperation {
    Read = 1,
    Write = 2,
    /// The transport adapter must complete this only after its remote fence.
    Flush = 3,
    List = 4,
    Stream = 5,
}

impl IoOperation {
    pub const fn raw(self) -> u16 {
        self as u16
    }

    fn rights(self) -> StorageRights {
        match self {
            Self::Read | Self::List => StorageRights::READ,
            Self::Write | Self::Flush => StorageRights::WRITE,
            Self::Stream => StorageRights::STREAM,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoRequest {
    pub id: u64,
    pub operation: IoOperation,
    pub path: StoragePath,
    pub length: usize,
    pub mount: MountId,
    pub buffer: Option<SharedBuffer>,
    pub buffer_capability: Option<BufferCapability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Completion {
    pub id: u64,
    pub operation: IoOperation,
    pub bytes: usize,
    pub status: Result<(), StorageError>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageBatchReport {
    pub requests: usize,
    pub bytes: usize,
    pub interrupt: bool,
    pub fairness_yield: bool,
    pub worker_placement: NumaDecision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountError {
    Capacity,
    Duplicate,
    InvalidLogical,
    InvalidOptions,
    InvalidProtocol,
    NotFound,
    Busy,
    Capability(CapabilityError),
    Endpoint(EndpointError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    InvalidPath,
    BufferTooLarge,
    QueueFull,
    NotFound,
    ReadOnly,
    Capability(CapabilityError),
    Mount(MountError),
    StreamFailed(u16),
    Buffer(BufferError),
}

pub struct StorageDaemon {
    secret: u64,
    mounts: [Option<MountInfo>; MAX_MOUNTS],
    next_mount: u32,
    catalog_version: u64,
    next_request: u64,
    pending: [Option<IoRequest>; MAX_PENDING_IO],
    completions: [Option<Completion>; MAX_PENDING_IO],
    controller: BatchController,
    numa: NumaPlacement,
    worker_cpu: Option<u16>,
    worker_node: Option<u8>,
}

impl StorageDaemon {
    pub const fn new(secret: u64) -> Self {
        Self {
            secret: if secret == 0 { 1 } else { secret },
            mounts: [None; MAX_MOUNTS],
            next_mount: ADMIN_MOUNT_ID + 1,
            catalog_version: 0,
            next_request: 1,
            pending: [None; MAX_PENDING_IO],
            completions: [None; MAX_PENDING_IO],
            controller: BatchController::new(ProducerPolicy::STORAGE),
            numa: NumaPlacement::uma(),
            worker_cpu: None,
            worker_node: None,
        }
    }

    pub fn configure_numa(
        &mut self,
        topology: NumaTopology,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        self.numa.set_topology(topology);
        self.worker_cpu = preferred_cpu;
        self.worker_node = preferred_node;
        let placement = self
            .numa
            .place(PlacementKind::StorageWorker, preferred_cpu, preferred_node);
        synos_observability::info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::StorageWorker as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, placement.locality as u64),
        );
        placement
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.numa.report()
    }

    pub fn bootstrap_capability(
        &self,
        expires_at: u64,
    ) -> Result<StorageCapability, MountError> {
        StorageCapability::issue(
            ADMIN_MOUNT_ID,
            1,
            StorageRights::ALL,
            StoragePath::ROOT,
            expires_at,
            self.secret,
        )
        .map_err(MountError::Capability)
    }

    pub fn mount(
        &mut self,
        admin: StorageCapability,
        endpoint: Endpoint,
        protocol: Protocol,
        logical: StoragePath,
        options: MountOptions,
        now: u64,
    ) -> Result<MountInfo, MountError> {
        self.verify_admin(admin, now)?;
        options.validate()?;
        if logical == StoragePath::ROOT || logical.mount_name().is_empty() {
            return Err(MountError::InvalidLogical)
        }
        if self
            .mounts
            .iter()
            .flatten()
            .any(|mount| mount.logical == logical)
        {
            return Err(MountError::Duplicate)
        }
        let slot = self
            .mounts
            .iter_mut()
            .find(|mount| mount.is_none())
            .ok_or(MountError::Capacity)?;
        let id = MountId(self.next_mount);
        self.next_mount = self.next_mount.checked_add(1).ok_or(MountError::Capacity)?;
        self.catalog_version = self.catalog_version.saturating_add(1);
        let info = MountInfo {
            id,
            logical,
            endpoint,
            protocol,
            features: protocol.features(),
            options,
            generation: 1,
            catalog_version: self.catalog_version,
            state: MountState::Mounted,
        };
        *slot = Some(info);
        Ok(info)
    }

    pub fn unmount(
        &mut self,
        admin: StorageCapability,
        id: MountId,
        now: u64,
    ) -> Result<(), MountError> {
        self.verify_admin(admin, now)?;
        let slot = self
            .mounts
            .iter_mut()
            .find(|mount| mount.is_some_and(|mount| mount.id == id))
            .ok_or(MountError::NotFound)?;
        *slot = None;
        self.catalog_version = self.catalog_version.saturating_add(1);
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Storage,
            CapabilityTraceStage::Revoked,
            admin.trace_id(),
            2,
        ) {
            trace.emit(Level::Info)
        }
        Ok(())
    }

    pub fn list_mounts(&self, output: &mut [MountInfo]) -> usize {
        let mut written = 0;
        for mount in self.mounts.iter().flatten() {
            if let Some(destination) = output.get_mut(written) {
                *destination = *mount;
                written += 1
            } else {
                break
            }
        }
        written
    }

    pub fn capability(
        &self,
        admin: StorageCapability,
        id: MountId,
        rights: StorageRights,
        scope: StoragePath,
        expires_at: u64,
        now: u64,
    ) -> Result<StorageCapability, MountError> {
        self.verify_admin(admin, now)?;
        let mount = self.find_mount(id).ok_or(MountError::NotFound)?;
        if !mount.logical.contains(scope) {
            return Err(MountError::InvalidLogical)
        }
        StorageCapability::issue(
            id.raw(),
            mount.generation,
            rights,
            scope,
            expires_at,
            self.secret,
        )
        .map_err(MountError::Capability)
    }

    pub fn submit_io(
        &mut self,
        capability: StorageCapability,
        operation: IoOperation,
        path: StoragePath,
        length: usize,
        now: u64,
    ) -> Result<IoRequest, StorageError> {
        let mount = self
            .mounts
            .iter()
            .flatten()
            .find(|mount| mount.id.raw() == capability.mount())
            .copied()
            .ok_or(StorageError::NotFound)?;
        if length > mount.options.max_io_bytes {
            return Err(StorageError::BufferTooLarge)
        }
        if mount.options.read_only && matches!(operation, IoOperation::Write | IoOperation::Flush) {
            return Err(StorageError::ReadOnly)
        }
        capability
            .verify(
                mount.id.raw(),
                mount.generation,
                path,
                operation.rights(),
                now,
                self.secret,
            )
            .map_err(StorageError::Capability)?;
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Storage,
            CapabilityTraceStage::KernelIpc,
            capability.trace_id(),
            operation.raw(),
        ) {
            trace.emit(Level::Trace)
        }
        let slot = self
            .pending
            .iter_mut()
            .find(|request| request.is_none())
            .ok_or(StorageError::QueueFull)?;
        let id = self.next_request;
        self.next_request = self.next_request.saturating_add(1);
        let request = IoRequest {
            id,
            operation,
            path,
            length,
            mount: mount.id,
            buffer: None,
            buffer_capability: None,
        };
        *slot = Some(request);
        Ok(request)
    }

    pub fn submit_io_buffer(
        &mut self,
        capability: StorageCapability,
        operation: IoOperation,
        path: StoragePath,
        buffer: &BufferLease<'_>,
        now: u64,
    ) -> Result<IoRequest, StorageError> {
        if buffer.owner() != BufferOwner::Storage {
            return Err(StorageError::Buffer(BufferError::OwnerMismatch));
        }
        let required = match operation {
            IoOperation::Read => BufferRights::WRITE,
            IoOperation::Write | IoOperation::Stream => BufferRights::READ,
            IoOperation::Flush | IoOperation::List => BufferRights::READ,
        };
        buffer
            .capability()
            .authorize(buffer.descriptor(), required)
            .map_err(StorageError::Buffer)?;
        let mut request = self.submit_io(
            capability,
            operation,
            path,
            buffer.descriptor().length as usize,
            now,
        )?;
        request.buffer = Some(buffer.descriptor());
        request.buffer_capability = Some(buffer.capability());
        if let Some(slot) = self
            .pending
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.id == request.id))
        {
            *slot = Some(request);
        }
        Ok(request)
    }

    /// Drive one request after a transport adapter has submitted its remote
    /// operation. Real device code can replace this with protocol completions.
    pub fn complete_next(&mut self, status: Result<(), StorageError>, bytes: usize) -> Option<Completion> {
        let request = self.pending.iter_mut().find_map(Option::take)?;
        let completion = Completion {
            id: request.id,
            operation: request.operation,
            bytes: bytes.min(request.length),
            status,
        };
        if let Some(slot) = self.completions.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(completion);
            Some(completion)
        } else {
            Some(Completion {
                status: Err(StorageError::QueueFull),
                ..completion
            })
        }
    }

    /// Take one bounded batch for a storage adapter. Taking ownership prevents
    /// a request from being dispatched twice. The adapter must later report
    /// each request through its normal completion path.
    pub fn take_io_batch(
        &mut self,
        output: &mut [Option<IoRequest>],
        _now_us: u64,
        interactive_pending: bool,
    ) -> StorageBatchReport {
        output.fill(None);
        let pending = self.pending.iter().flatten().count();
        let decision = self.controller.plan(pending, 1, 0, interactive_pending);
        let max_bytes = self.controller.policy().max_bytes;
        let limit = decision.count.min(output.len()).min(MAX_STORAGE_BATCH);
        let worker_placement = self.numa.place(
            PlacementKind::StorageWorker,
            self.worker_cpu,
            self.worker_node,
        );
        synos_observability::info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::StorageWorker as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, worker_placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, worker_placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, worker_placement.locality as u64),
        );
        let mut report = StorageBatchReport {
            interrupt: decision.interrupt,
            fairness_yield: decision.fairness_yield,
            worker_placement,
            ..StorageBatchReport::default()
        };
        for slot in &mut self.pending {
            if report.requests >= limit {
                break
            }
            let Some(request) = *slot else { continue };
            let next_bytes = report.bytes.saturating_add(request.length);
            if next_bytes > max_bytes && report.requests != 0 {
                break
            }
            *slot = None;
            output[report.requests] = Some(request);
            report.requests += 1;
            report.bytes = next_bytes;
        }
        report
    }

    pub fn poll_completion(&mut self) -> Option<Completion> {
        self.completions.iter_mut().find_map(Option::take)
    }

    pub fn mount_catalog(&self) -> crate::MountCatalog {
        crate::MountCatalog::from_mounts(self.mounts, self.catalog_version)
    }

    pub fn restore_catalog(&mut self, catalog: crate::MountCatalog) -> Result<(), MountError> {
        self.mounts = catalog.mounts();
        self.catalog_version = catalog.version();
        self.next_mount = self
            .mounts
            .iter()
            .flatten()
            .map(|mount| mount.id.raw())
            .max()
            .unwrap_or(ADMIN_MOUNT_ID)
            .saturating_add(1);
        Ok(())
    }

    fn verify_admin(&self, capability: StorageCapability, now: u64) -> Result<(), MountError> {
        capability
            .verify(
                ADMIN_MOUNT_ID,
                1,
                StoragePath::ROOT,
                StorageRights::ADMIN,
                now,
                self.secret,
            )
            .map_err(MountError::Capability)
    }

    fn find_mount(&self, id: MountId) -> Option<MountInfo> {
        self.mounts.iter().flatten().find(|mount| mount.id == id).copied()
    }

}

impl From<EndpointError> for MountError {
    fn from(error: EndpointError) -> Self {
        Self::Endpoint(error)
    }
}
