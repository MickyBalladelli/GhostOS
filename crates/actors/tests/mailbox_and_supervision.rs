// Inventory: coverage_59_6.rs (legacy roadmap section 59).
use ghostos_actors::{
    ActorEndpoint, ActorError, ActorId, ActorMessage, ActorSystem, ActorTransport, DsmMailbox,
};
use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE, dsm::RemotePageAuthority};
use ghostos_ipc::{ChannelId, Envelope, SharedBuffer};
use ghostos_status::{IntoStatus, Status};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportError {
    Failed,
}

impl IntoStatus for TransportError {
    fn status(self) -> Status {
        Status::BUSY
    }
}

#[derive(Default)]
struct Transport {
    sent: Vec<(ChannelId, Envelope)>,
    remote: Vec<ghostos_actors::DsmDelivery>,
    now: u64,
}

impl ActorTransport for Transport {
    type Error = TransportError;

    fn now_us(&self) -> u64 {
        self.now
    }

    fn send_ipc(&mut self, channel: ChannelId, envelope: Envelope) -> Result<(), Self::Error> {
        self.sent.push((channel, envelope));
        Ok(())
    }

    fn receive_ipc(&mut self, _channel: ChannelId) -> Result<Option<Envelope>, Self::Error> {
        Ok(None)
    }

    fn send_dsm(&mut self, delivery: ghostos_actors::DsmDelivery) -> Result<(), Self::Error> {
        self.remote.push(delivery);
        Ok(())
    }
}

fn actor(node: NodeId, local: u64) -> ActorId {
    ActorId::new(node, local).unwrap()
}

#[test]
fn actor_messages_round_trip_and_reject_corrupt_identity() {
    let source = actor(NodeId::LOCAL, 1);
    let destination = actor(NodeId::new(2).unwrap(), 2);
    let message = ActorMessage {
        source,
        destination,
        correlation: 55,
        label: 9,
        payload: Some(SharedBuffer {
            region: ghostos_ipc::SharedRegionId::new(3).unwrap(),
            offset: 4,
            length: 8,
            writable: false,
        }),
    };
    assert_eq!(ActorMessage::decode(message.encode()).unwrap(), message);
    let mut corrupt = message.encode();
    corrupt.words[1] = 0;
    assert_eq!(ActorMessage::decode(corrupt), Err(ActorError::CorruptEnvelope));
}

#[test]
fn actor_system_routes_local_and_remote_messages_with_mailbox_checks() {
    let local_actor = actor(NodeId::LOCAL, 1);
    let remote_node = NodeId::new(2).unwrap();
    let remote_range = AddressRange::new(PAGE_SIZE, PAGE_SIZE).unwrap();
    let mailbox = DsmMailbox {
        destination: remote_node,
        range: remote_range,
        authority: RemotePageAuthority {
            subject: NodeId::LOCAL,
            range: remote_range,
            read: true,
            write: true,
            lease_epoch: 1,
            expires_at_us: 100,
        },
    };
    mailbox.validate(NodeId::LOCAL, 10).unwrap();
    assert!(mailbox.validate(NodeId::LOCAL, 100).is_err());

    let mut system = ActorSystem::<4, 2>::new(NodeId::LOCAL);
    let local_ref = system
        .register(local_actor, ActorEndpoint::Local(ChannelId::new(7).unwrap()), 1)
        .unwrap();
    assert_eq!(system.resolve(local_ref).unwrap().1, 1);
    let remote_actor = actor(remote_node, 2);
    let remote_ref = system
        .register(remote_actor, ActorEndpoint::Remote(mailbox), 1)
        .unwrap();
    let mut transport = Transport { now: 10, ..Transport::default() };
    system
        .send(
            &mut transport,
            ActorMessage {
                source: local_actor,
                destination: local_actor,
                correlation: 1,
                label: 2,
                payload: None,
            },
        )
        .unwrap();
    assert_eq!(transport.sent.len(), 1);
    system
        .send(
            &mut transport,
            ActorMessage {
                source: local_actor,
                destination: remote_actor,
                correlation: 3,
                label: 4,
                payload: None,
            },
        )
        .unwrap();
    assert_eq!(transport.remote.len(), 1);
    assert_eq!(system.forget(remote_ref), Ok(()));
    assert_eq!(system.resolve(remote_ref), Err(ActorError::NotFound));
}
