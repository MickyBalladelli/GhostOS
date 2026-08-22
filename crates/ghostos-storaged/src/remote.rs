use crate::protocol::{BlockTransport, Endpoint, FabricTransport, ObjectKey, S3Range};

pub const MAX_PNFS_DATA_SERVERS: usize = 8;
pub const MAX_SMB_CHANNELS: usize = 8;
pub const MAX_NVME_QUEUE_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PnfsDataServer {
    pub endpoint: Endpoint,
    pub stripe_start: u64,
    pub stripe_length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PnfsLayout {
    servers: [Option<PnfsDataServer>; MAX_PNFS_DATA_SERVERS],
    count: u8,
    pub stripe_unit: u32,
}

impl PnfsLayout {
    pub const fn new(stripe_unit: u32) -> Option<Self> {
        if stripe_unit == 0 {
            None
        } else {
            Some(Self {
                servers: [None; MAX_PNFS_DATA_SERVERS],
                count: 0,
                stripe_unit,
            })
        }
    }

    pub fn add_server(&mut self, server: PnfsDataServer) -> Result<(), PnfsError> {
        if server.stripe_length == 0 {
            return Err(PnfsError::InvalidLayout)
        }
        let slot = self
            .servers
            .iter_mut()
            .find(|server| server.is_none())
            .ok_or(PnfsError::Capacity)?;
        *slot = Some(server);
        self.count += 1;
        Ok(())
    }

    pub fn server_for(&self, offset: u64) -> Option<PnfsDataServer> {
        self.servers
            .iter()
            .flatten()
            .find(|server| offset >= server.stripe_start && offset < server.stripe_start + server.stripe_length)
            .copied()
    }

    pub const fn server_count(&self) -> usize {
        self.count as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PnfsClient {
    pub endpoint: Endpoint,
    pub minor: crate::NfsMinorVersion,
    pub layout: Option<PnfsLayout>,
}

impl PnfsClient {
    pub const fn new(endpoint: Endpoint, minor: crate::NfsMinorVersion) -> Self {
        Self {
            endpoint,
            minor,
            layout: None,
        }
    }

    pub fn install_layout(&mut self, layout: PnfsLayout) -> Result<(), PnfsError> {
        if layout.server_count() == 0 {
            return Err(PnfsError::InvalidLayout)
        }
        self.layout = Some(layout);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PnfsError {
    Capacity,
    InvalidLayout,
    NoLayout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SmbChannel {
    pub id: u8,
    pub transport: BlockTransport,
    pub credits: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmbSessionState {
    New,
    Negotiated,
    Authenticated,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SmbSession {
    pub endpoint: Endpoint,
    pub state: SmbSessionState,
    channels: [Option<SmbChannel>; MAX_SMB_CHANNELS],
    channel_count: u8,
    pub multichannel: bool,
    pub direct: bool,
}

impl SmbSession {
    pub const fn new(endpoint: Endpoint, multichannel: bool, direct: bool) -> Self {
        Self {
            endpoint,
            state: SmbSessionState::New,
            channels: [None; MAX_SMB_CHANNELS],
            channel_count: 0,
            multichannel,
            direct,
        }
    }

    pub fn negotiate(&mut self) -> Result<(), SmbError> {
        if self.state != SmbSessionState::New {
            return Err(SmbError::InvalidState)
        }
        self.state = SmbSessionState::Negotiated;
        Ok(())
    }

    pub fn authenticate(&mut self) -> Result<(), SmbError> {
        if self.state != SmbSessionState::Negotiated {
            return Err(SmbError::InvalidState)
        }
        self.state = SmbSessionState::Authenticated;
        Ok(())
    }

    pub fn add_channel(&mut self, channel: SmbChannel) -> Result<(), SmbError> {
        if self.state != SmbSessionState::Authenticated || (!self.multichannel && self.channel_count != 0) {
            return Err(SmbError::InvalidState)
        }
        if self.direct && channel.transport != BlockTransport::Rdma {
            return Err(SmbError::TransportUnavailable)
        }
        let slot = self
            .channels
            .iter_mut()
            .find(|channel| channel.is_none())
            .ok_or(SmbError::Capacity)?;
        *slot = Some(channel);
        self.channel_count += 1;
        Ok(())
    }

    pub const fn channel_count(&self) -> usize {
        self.channel_count as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmbError {
    Capacity,
    InvalidState,
    TransportUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NvmeTarget {
    pub endpoint: Endpoint,
    pub transport: FabricTransport,
    pub namespace_id: u32,
    pub queue_count: u16,
}

impl NvmeTarget {
    pub const fn new(
        endpoint: Endpoint,
        transport: FabricTransport,
        namespace_id: u32,
        queue_count: u16,
    ) -> Option<Self> {
        if namespace_id == 0 || queue_count == 0 {
            None
        } else {
            Some(Self {
                endpoint,
                transport,
                namespace_id,
                queue_count,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NvmeCommand {
    pub command_id: u16,
    pub offset: u64,
    pub length: u32,
    pub write: bool,
}

pub struct NvmeQueue<const DEPTH: usize = MAX_NVME_QUEUE_DEPTH> {
    commands: [Option<NvmeCommand>; DEPTH],
}

impl<const DEPTH: usize> NvmeQueue<DEPTH> {
    pub const fn new() -> Self {
        Self {
            commands: [None; DEPTH],
        }
    }

    pub fn submit(&mut self, command: NvmeCommand) -> Result<(), NvmeError> {
        let slot = self
            .commands
            .iter_mut()
            .find(|command| command.is_none())
            .ok_or(NvmeError::QueueFull)?;
        *slot = Some(command);
        Ok(())
    }

    pub fn complete(&mut self, command_id: u16) -> Option<NvmeCommand> {
        self.commands
            .iter_mut()
            .find(|command| command.is_some_and(|value| value.command_id == command_id))
            .and_then(Option::take)
    }
}

impl<const DEPTH: usize> Default for NvmeQueue<DEPTH> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NvmeError {
    QueueFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IscsiState {
    New,
    LoggedIn,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IscsiSession {
    pub endpoint: Endpoint,
    pub state: IscsiState,
    pub target: ObjectKey,
    pub lun: u64,
}

impl IscsiSession {
    pub const fn new(endpoint: Endpoint, target: ObjectKey, lun: u64) -> Option<Self> {
        if lun == u64::MAX {
            None
        } else {
            Some(Self {
                endpoint,
                state: IscsiState::New,
                target,
                lun,
            })
        }
    }

    pub fn login(&mut self) -> Result<(), IscsiError> {
        if self.state != IscsiState::New {
            return Err(IscsiError::InvalidState)
        }
        self.state = IscsiState::LoggedIn;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IscsiError {
    InvalidState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct S3GetRequest {
    pub bucket: ObjectKey,
    pub object: ObjectKey,
    pub range: Option<S3Range>,
    pub resumable: bool,
}
