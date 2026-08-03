#![no_std]
#![forbid(unsafe_code)]

mod cache;
mod capability;
mod cluster;
mod protocol;
mod remote;
mod service;
mod state;

pub use cache::{CacheError, CacheMode, CowCache, RemoteFileBackend};
pub use capability::{CapabilityError, StorageCapability, StorageRights};
pub use cluster::{
    Certificate, ClusterId, ClusterLifecycle, ClusterMetadata, ClusterMetadataCatalog,
    ClusterMetadataError, ClusterMetadataSnapshot, ClusterPatch, Invitation, MembershipIntent,
    MetadataText, TrustedPeer, CLUSTER_METADATA_FORMAT_VERSION, CLUSTER_METADATA_MAGIC,
    CLUSTER_METADATA_STATE_FILE, CLUSTER_ID_BYTES, MAX_CERTIFICATES, MAX_CLUSTER_ALIASES,
    MAX_CLUSTER_DESCRIPTION_BYTES, MAX_CLUSTER_NAME_BYTES, MAX_CLUSTERS, MAX_INVITATIONS,
    MAX_TRUSTED_PEERS,
};
pub use protocol::{
    BlockTransport, Endpoint, EndpointError, FabricTransport, NfsMinorVersion, ObjectKey,
    Protocol, ProtocolFeatures, S3Range, SmbDialect,
};
pub use remote::{
    IscsiSession, IscsiState, NvmeCommand, NvmeQueue, NvmeTarget, PnfsClient, PnfsDataServer,
    PnfsLayout, S3GetRequest, SmbChannel, SmbSession, SmbSessionState,
};
pub use service::{
    Completion, IoOperation, IoRequest, MountError, MountId, MountInfo, MountOptions,
    MountState, StorageDaemon, StorageError, StoragePath, MAX_MOUNTS, MAX_PENDING_IO,
};
pub use state::{MountCatalog, MountStateError, MOUNTS_STATE_FILE, MOUNTS_STATE_MAGIC};

/// Logical storage namespace used by remote mounts.
pub const STORAGE_LOGICAL: &str = "SYS$STORAGE:";

/// Maximum bytes accepted by one remote data operation.
pub const MAX_IO_BYTES: usize = 1024 * 1024;

/// Maximum bytes accepted by one S3 stream segment.
pub const MAX_S3_SEGMENT_BYTES: usize = 1024 * 1024;

/// A netd-owned stream that can hand network buffers to a storage consumer
/// without making the storage daemon own or copy those buffers.
pub trait NetworkBufferStream {
    fn next<'a>(&'a mut self) -> Option<&'a [u8]>;
}

/// Consume S3 data directly from the zero-copy ingress queue owned by
/// `synos-netd`. Each packet is loaned only for the duration of `accept`.
pub fn stream_s3_netd<const CAPACITY: usize, const MTU: usize>(
    queue: &mut synos_netd::PacketQueue<CAPACITY, MTU>,
    sink: &mut impl S3StreamSink,
) -> Result<u64, StorageError> {
    let mut total = 0u64;
    while let Ok(packet) = queue.dequeue() {
        let buffer = packet.frame();
        sink.accept(buffer).map_err(StorageError::StreamFailed)?;
        total = total.saturating_add(buffer.len() as u64);
    }
    sink.finish().map_err(StorageError::StreamFailed)?;
    Ok(total)
}

/// Consume an S3 body using borrowed buffers supplied by `synos-netd`.
pub fn stream_s3_body(
    stream: &mut impl NetworkBufferStream,
    sink: &mut impl S3StreamSink,
) -> Result<u64, StorageError> {
    let mut total = 0u64;
    while let Some(buffer) = stream.next() {
        if buffer.len() > MAX_S3_SEGMENT_BYTES {
            return Err(StorageError::BufferTooLarge)
        }
        sink.accept(buffer).map_err(StorageError::StreamFailed)?;
        total = total.saturating_add(buffer.len() as u64);
    }
    sink.finish().map_err(StorageError::StreamFailed)?;
    Ok(total)
}

pub trait S3StreamSink {
    fn accept(&mut self, buffer: &[u8]) -> Result<(), u16>;
    fn finish(&mut self) -> Result<(), u16>;
}
