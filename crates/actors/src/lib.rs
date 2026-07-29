#![no_std]
#![forbid(unsafe_code)]

use synos_fabric::{AddressRange, NodeId, PAGE_SIZE, dsm::RemotePageAuthority};
use synos_ipc::{ChannelId, Envelope, SharedBuffer};
use synos_status::{IntoStatus, Status};

pub const DEFAULT_ACTOR_CAPACITY: usize = 128;
pub const DEFAULT_NODE_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ActorId {
    pub node: NodeId,
    pub local: u64,
}

impl ActorId {
    pub const fn new(node: NodeId, local: u64) -> Option<Self> {
        if local == 0 {
            None
        } else {
            Some(Self { node, local })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActorRef {
    id: ActorId,
}

impl ActorRef {
    pub const fn new(id: ActorId) -> Self {
        Self { id }
    }

    pub const fn id(self) -> ActorId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmMailbox {
    pub destination: NodeId,
    pub range: AddressRange,
    pub authority: RemotePageAuthority,
}

impl DsmMailbox {
    pub fn validate(
        self,
        local: NodeId,
        now_us: u64,
    ) -> Result<(), ActorError<core::convert::Infallible>> {
        let last = self
            .range
            .end()
            .checked_sub(1)
            .ok_or(ActorError::InvalidMailbox)?;
        if self.destination == local
            || self.range.start % PAGE_SIZE != 0
            || self.range.length % PAGE_SIZE != 0
            || self.authority.subject != local
            || !self.authority.write
            || !self.authority.range.contains(self.range.start)
            || !self.authority.range.contains(last)
            || self.authority.lease_epoch == 0
            || now_us >= self.authority.expires_at_us
        {
            return Err(ActorError::InvalidMailbox);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorEndpoint {
    Local(ChannelId),
    Remote(DsmMailbox),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActorMessage {
    pub source: ActorId,
    pub destination: ActorId,
    pub correlation: u128,
    pub label: u64,
    pub payload: Option<SharedBuffer>,
}

impl ActorMessage {
    pub const fn encode(self) -> Envelope {
        Envelope {
            correlation: self.correlation,
            label: self.label,
            buffer: self.payload,
            words: [
                self.source.node.raw() as u64,
                self.source.local,
                self.destination.node.raw() as u64,
                self.destination.local,
            ],
        }
    }

    pub fn decode(envelope: Envelope) -> Result<Self, ActorError<core::convert::Infallible>> {
        let source_node =
            NodeId::new(envelope.words[0] as u32).ok_or(ActorError::CorruptEnvelope)?;
        let destination_node =
            NodeId::new(envelope.words[2] as u32).ok_or(ActorError::CorruptEnvelope)?;
        if envelope.words[0] > u32::MAX as u64
            || envelope.words[2] > u32::MAX as u64
            || envelope.words[1] == 0
            || envelope.words[3] == 0
        {
            return Err(ActorError::CorruptEnvelope);
        }
        Ok(Self {
            source: ActorId {
                node: source_node,
                local: envelope.words[1],
            },
            destination: ActorId {
                node: destination_node,
                local: envelope.words[3],
            },
            correlation: envelope.correlation,
            label: envelope.label,
            payload: envelope.buffer,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmDelivery {
    pub mailbox: DsmMailbox,
    pub envelope: Envelope,
}

pub trait ActorTransport {
    type Error: IntoStatus;

    fn now_us(&self) -> u64;
    fn send_ipc(&mut self, channel: ChannelId, envelope: Envelope) -> Result<(), Self::Error>;
    fn receive_ipc(&mut self, channel: ChannelId) -> Result<Option<Envelope>, Self::Error>;
    fn send_dsm(&mut self, delivery: DsmDelivery) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActorSpawnRequest {
    pub actor: ActorId,
    pub image: u128,
    pub capability_profile: u64,
    pub generation: u32,
}

pub trait ActorRuntime: ActorTransport {
    fn spawn_local(&mut self, request: ActorSpawnRequest) -> Result<ActorEndpoint, Self::Error>;
    fn spawn_remote(
        &mut self,
        supervisor: DsmMailbox,
        request: ActorSpawnRequest,
    ) -> Result<ActorEndpoint, Self::Error>;
    fn stop_actor(&mut self, actor: ActorId, endpoint: ActorEndpoint) -> Result<(), Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum ActorError<E> {
    ActorFailed(Status),
    AlreadyRegistered,
    Capacity,
    CorruptEnvelope,
    InvalidActor,
    InvalidEndpoint,
    InvalidMailbox,
    NodeRouteMissing,
    NotFound,
    Transport(E),
}

impl<E: IntoStatus> IntoStatus for ActorError<E> {
    fn status(self) -> Status {
        match self {
            Self::ActorFailed(status) => status,
            Self::Transport(error) => error.status(),
            Self::AlreadyRegistered | Self::InvalidActor | Self::InvalidEndpoint => {
                Status::INVALID_ARGUMENT
            }
            Self::Capacity => Status::NO_SPACE,
            Self::NotFound | Self::NodeRouteMissing => Status::NOT_FOUND,
            Self::CorruptEnvelope => Status::CORRUPT,
            Self::InvalidMailbox => Status::ACCESS_DENIED,
        }
    }
}

#[derive(Clone, Copy)]
struct DirectoryEntry {
    actor: ActorId,
    endpoint: ActorEndpoint,
    generation: u32,
}

#[derive(Clone, Copy)]
struct NodeRoute {
    node: NodeId,
    supervisor: DsmMailbox,
}

pub struct ActorSystem<
    const ACTORS: usize = DEFAULT_ACTOR_CAPACITY,
    const NODES: usize = DEFAULT_NODE_CAPACITY,
> {
    local: NodeId,
    directory: [Option<DirectoryEntry>; ACTORS],
    routes: [Option<NodeRoute>; NODES],
}

impl<const ACTORS: usize, const NODES: usize> ActorSystem<ACTORS, NODES> {
    pub const fn new(local: NodeId) -> Self {
        Self {
            local,
            directory: [None; ACTORS],
            routes: [None; NODES],
        }
    }

    pub const fn local_node(&self) -> NodeId {
        self.local
    }

    pub fn register(
        &mut self,
        actor: ActorId,
        endpoint: ActorEndpoint,
        generation: u32,
    ) -> Result<ActorRef, ActorError<core::convert::Infallible>> {
        if generation == 0 || !endpoint_matches(actor, endpoint, self.local) {
            return Err(ActorError::InvalidEndpoint);
        }
        if self
            .directory
            .iter()
            .flatten()
            .any(|entry| entry.actor == actor)
        {
            return Err(ActorError::AlreadyRegistered);
        }
        let slot = self
            .directory
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ActorError::Capacity)?;
        *slot = Some(DirectoryEntry {
            actor,
            endpoint,
            generation,
        });
        Ok(ActorRef::new(actor))
    }

    pub fn add_node_route(
        &mut self,
        route: DsmMailbox,
        now_us: u64,
    ) -> Result<(), ActorError<core::convert::Infallible>> {
        route.validate(self.local, now_us)?;
        if self
            .routes
            .iter()
            .flatten()
            .any(|entry| entry.node == route.destination)
        {
            return Err(ActorError::AlreadyRegistered);
        }
        let slot = self
            .routes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ActorError::Capacity)?;
        *slot = Some(NodeRoute {
            node: route.destination,
            supervisor: route,
        });
        Ok(())
    }

    pub fn resolve(
        &self,
        actor: ActorRef,
    ) -> Result<(ActorEndpoint, u32), ActorError<core::convert::Infallible>> {
        let entry = self
            .directory
            .iter()
            .flatten()
            .find(|entry| entry.actor == actor.id)
            .ok_or(ActorError::NotFound)?;
        Ok((entry.endpoint, entry.generation))
    }

    pub fn send<T: ActorTransport>(
        &self,
        transport: &mut T,
        message: ActorMessage,
    ) -> Result<(), ActorError<T::Error>> {
        let entry = self
            .directory
            .iter()
            .flatten()
            .find(|entry| entry.actor == message.destination)
            .ok_or(ActorError::NotFound)?;
        match entry.endpoint {
            ActorEndpoint::Local(channel) => transport
                .send_ipc(channel, message.encode())
                .map_err(ActorError::Transport),
            ActorEndpoint::Remote(mailbox) => {
                mailbox
                    .validate(self.local, transport.now_us())
                    .map_err(convert_infallible)?;
                transport
                    .send_dsm(DsmDelivery {
                        mailbox,
                        envelope: message.encode(),
                    })
                    .map_err(ActorError::Transport)
            }
        }
    }

    pub fn poll<A: Actor, T: ActorTransport>(
        &self,
        actor_ref: ActorRef,
        actor: &mut A,
        transport: &mut T,
    ) -> Result<ActorPoll, ActorError<T::Error>> {
        let (endpoint, _) = self.resolve(actor_ref).map_err(convert_infallible)?;
        let ActorEndpoint::Local(channel) = endpoint else {
            return Err(ActorError::InvalidEndpoint);
        };
        let Some(envelope) = transport
            .receive_ipc(channel)
            .map_err(ActorError::Transport)?
        else {
            return Ok(ActorPoll::Idle);
        };
        let message = ActorMessage::decode(envelope).map_err(convert_infallible)?;
        if message.destination != actor_ref.id {
            return Err(ActorError::CorruptEnvelope);
        }
        let mut context = ActorContext {
            system: self,
            transport,
            actor: actor_ref,
        };
        actor
            .receive(message, &mut context)
            .map_err(ActorError::ActorFailed)?;
        Ok(ActorPoll::Handled {
            correlation: message.correlation,
        })
    }

    pub fn orchestrate<T: ActorRuntime>(
        &mut self,
        transport: &mut T,
        request: ActorSpawnRequest,
    ) -> Result<ActorRef, ActorError<T::Error>> {
        if request.actor.local == 0 || request.image == 0 || request.generation == 0 {
            return Err(ActorError::InvalidActor);
        }
        if self
            .directory
            .iter()
            .flatten()
            .any(|entry| entry.actor == request.actor)
        {
            return Err(ActorError::AlreadyRegistered);
        }
        let directory_slot = self
            .directory
            .iter()
            .position(|entry| entry.is_none())
            .ok_or(ActorError::Capacity)?;
        let endpoint = if request.actor.node == self.local {
            transport
                .spawn_local(request)
                .map_err(ActorError::Transport)?
        } else {
            let route = self
                .routes
                .iter()
                .flatten()
                .find(|route| route.node == request.actor.node)
                .ok_or(ActorError::NodeRouteMissing)?;
            route
                .supervisor
                .validate(self.local, transport.now_us())
                .map_err(convert_infallible)?;
            transport
                .spawn_remote(route.supervisor, request)
                .map_err(ActorError::Transport)?
        };
        if !endpoint_matches(request.actor, endpoint, self.local) {
            transport
                .stop_actor(request.actor, endpoint)
                .map_err(ActorError::Transport)?;
            return Err(ActorError::InvalidEndpoint);
        }
        self.directory[directory_slot] = Some(DirectoryEntry {
            actor: request.actor,
            endpoint,
            generation: request.generation,
        });
        Ok(ActorRef::new(request.actor))
    }

    pub fn stop<T: ActorRuntime>(
        &mut self,
        transport: &mut T,
        actor: ActorRef,
    ) -> Result<(), ActorError<T::Error>> {
        let index = self
            .directory
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.actor == actor.id))
            .ok_or(ActorError::NotFound)?;
        let entry = self.directory[index].expect("located actor entry");
        transport
            .stop_actor(entry.actor, entry.endpoint)
            .map_err(ActorError::Transport)?;
        self.directory[index] = None;
        Ok(())
    }
}

impl<const ACTORS: usize, const NODES: usize> Default for ActorSystem<ACTORS, NODES> {
    fn default() -> Self {
        Self::new(NodeId::LOCAL)
    }
}

pub struct ActorContext<'a, T, const ACTORS: usize, const NODES: usize> {
    system: &'a ActorSystem<ACTORS, NODES>,
    transport: &'a mut T,
    actor: ActorRef,
}

impl<T: ActorTransport, const ACTORS: usize, const NODES: usize>
    ActorContext<'_, T, ACTORS, NODES>
{
    pub const fn actor(&self) -> ActorRef {
        self.actor
    }

    pub fn send(
        &mut self,
        destination: ActorRef,
        correlation: u128,
        label: u64,
        payload: Option<SharedBuffer>,
    ) -> Result<(), ActorError<T::Error>> {
        self.system.send(
            self.transport,
            ActorMessage {
                source: self.actor.id,
                destination: destination.id,
                correlation,
                label,
                payload,
            },
        )
    }

    pub fn reply(
        &mut self,
        request: ActorMessage,
        label: u64,
        payload: Option<SharedBuffer>,
    ) -> Result<(), ActorError<T::Error>> {
        self.send(
            ActorRef::new(request.source),
            request.correlation,
            label,
            payload,
        )
    }
}

pub trait Actor {
    fn receive<T: ActorTransport, const ACTORS: usize, const NODES: usize>(
        &mut self,
        message: ActorMessage,
        context: &mut ActorContext<'_, T, ACTORS, NODES>,
    ) -> Result<(), Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorPoll {
    Idle,
    Handled { correlation: u128 },
}

fn endpoint_matches(actor: ActorId, endpoint: ActorEndpoint, local: NodeId) -> bool {
    match endpoint {
        ActorEndpoint::Local(_) => actor.node == local,
        ActorEndpoint::Remote(mailbox) => actor.node != local && mailbox.destination == actor.node,
    }
}

fn convert_infallible<E>(error: ActorError<core::convert::Infallible>) -> ActorError<E> {
    match error {
        ActorError::ActorFailed(status) => ActorError::ActorFailed(status),
        ActorError::AlreadyRegistered => ActorError::AlreadyRegistered,
        ActorError::Capacity => ActorError::Capacity,
        ActorError::CorruptEnvelope => ActorError::CorruptEnvelope,
        ActorError::InvalidActor => ActorError::InvalidActor,
        ActorError::InvalidEndpoint => ActorError::InvalidEndpoint,
        ActorError::InvalidMailbox => ActorError::InvalidMailbox,
        ActorError::NodeRouteMissing => ActorError::NodeRouteMissing,
        ActorError::NotFound => ActorError::NotFound,
        ActorError::Transport(never) => match never {},
    }
}
