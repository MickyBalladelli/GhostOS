use core::fmt;

pub const MAX_ENDPOINT_HOST_BYTES: usize = 96;
pub const MAX_ENDPOINT_PATH_BYTES: usize = 192;
pub const MAX_OBJECT_KEY_BYTES: usize = 192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointError {
    EmptyHost,
    HostTooLong,
    InvalidHost,
    InvalidPath,
    PathTooLong,
    InvalidPort,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Endpoint {
    host: [u8; MAX_ENDPOINT_HOST_BYTES],
    host_len: u8,
    path: [u8; MAX_ENDPOINT_PATH_BYTES],
    path_len: u16,
    port: u16,
}

impl Endpoint {
    pub fn new(host: &str, path: &str, port: u16) -> Result<Self, EndpointError> {
        if host.is_empty() {
            return Err(EndpointError::EmptyHost)
        }
        if host.len() > MAX_ENDPOINT_HOST_BYTES {
            return Err(EndpointError::HostTooLong)
        }
        if host.bytes().any(|byte| byte == 0 || byte.is_ascii_whitespace()) {
            return Err(EndpointError::InvalidHost)
        }
        if path.is_empty() || !path.starts_with('/') || path.contains('\0') {
            return Err(EndpointError::InvalidPath)
        }
        if path.len() > MAX_ENDPOINT_PATH_BYTES {
            return Err(EndpointError::PathTooLong)
        }
        if port == 0 {
            return Err(EndpointError::InvalidPort)
        }
        let mut stored_host = [0; MAX_ENDPOINT_HOST_BYTES];
        stored_host[..host.len()].copy_from_slice(host.as_bytes());
        let mut stored_path = [0; MAX_ENDPOINT_PATH_BYTES];
        stored_path[..path.len()].copy_from_slice(path.as_bytes());
        Ok(Self {
            host: stored_host,
            host_len: host.len() as u8,
            path: stored_path,
            path_len: path.len() as u16,
            port,
        })
    }

    pub fn parse(value: &str, default_port: u16) -> Result<Self, EndpointError> {
        let (host, path) = value.split_once(':').ok_or(EndpointError::InvalidPath)?;
        let path = if path.starts_with('/') {
            path
        } else {
            return Err(EndpointError::InvalidPath)
        };
        Self::new(host, path, default_port)
    }

    pub fn host(&self) -> &str {
        core::str::from_utf8(&self.host[..self.host_len as usize]).unwrap_or("")
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len as usize]).unwrap_or("")
    }

    pub const fn port(self) -> u16 {
        self.port
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Endpoint")
            .field("host", &self.host())
            .field("path", &self.path())
            .field("port", &self.port)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NfsMinorVersion {
    V41,
    V42,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmbDialect {
    Smb311,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FabricTransport {
    Tcp,
    RoceV2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockTransport {
    Tcp,
    Rdma,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Pnfs(NfsMinorVersion),
    Smb {
        dialect: SmbDialect,
        multichannel: bool,
        direct: bool,
    },
    NvmeOf(FabricTransport),
    Iscsi,
    S3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolFeatures(u16);

impl ProtocolFeatures {
    pub const PARALLEL_LAYOUTS: Self = Self(1 << 0);
    pub const MULTICHANNEL: Self = Self(1 << 1);
    pub const RDMA: Self = Self(1 << 2);
    pub const WRITE_ZERO_COPY: Self = Self(1 << 3);
    pub const RESUMABLE_STREAMS: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl Protocol {
    pub const fn features(self) -> ProtocolFeatures {
        match self {
            Self::Pnfs(_) => ProtocolFeatures::PARALLEL_LAYOUTS,
            Self::Smb {
                multichannel,
                direct,
                ..
            } => {
                let mut features = ProtocolFeatures::empty();
                if multichannel {
                    features = features.union(ProtocolFeatures::MULTICHANNEL)
                }
                if direct {
                    features = features.union(ProtocolFeatures::RDMA)
                }
                features
            }
            Self::NvmeOf(FabricTransport::RoceV2) => ProtocolFeatures::RDMA,
            Self::NvmeOf(FabricTransport::Tcp) => ProtocolFeatures::empty(),
            Self::Iscsi => ProtocolFeatures::empty(),
            Self::S3 => ProtocolFeatures::WRITE_ZERO_COPY.union(ProtocolFeatures::RESUMABLE_STREAMS),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ObjectKey {
    bytes: [u8; MAX_OBJECT_KEY_BYTES],
    len: u16,
}

impl ObjectKey {
    pub fn new(key: &str) -> Result<Self, EndpointError> {
        if key.is_empty() || key.len() > MAX_OBJECT_KEY_BYTES || key.contains('\0') {
            return Err(EndpointError::InvalidPath)
        }
        let mut bytes = [0; MAX_OBJECT_KEY_BYTES];
        bytes[..key.len()].copy_from_slice(key.as_bytes());
        Ok(Self {
            bytes,
            len: key.len() as u16,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl fmt::Debug for ObjectKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ObjectKey").field(&self.as_str()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct S3Range {
    pub offset: u64,
    pub length: u64,
}

impl S3Range {
    pub const fn new(offset: u64, length: u64) -> Option<Self> {
        if length == 0 || offset.checked_add(length).is_none() {
            None
        } else {
            Some(Self { offset, length })
        }
    }
}
