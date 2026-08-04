use synos_client_sdk::{
    Client, ClientError, ClusterCreateRequest, ClusterId, ClusterJoinRequest,
    ClusterLeaveRequest, ClusterName, ClusterRemoveRequest, NodeId, ProtocolError, RpcTransport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransportFailure;

impl RpcTransport for TransportFailure {
    type Error = TransportFailure;

    fn round_trip(&mut self, _request: &[u8], _response: &mut [u8]) -> Result<usize, Self::Error> {
        Err(TransportFailure)
    }
}

fn create_request() -> ClusterCreateRequest {
    ClusterCreateRequest {
        cluster: ClusterId::new(7).unwrap(),
        name: ClusterName::new("compute").unwrap(),
    }
}

#[test]
fn lifecycle_api_preserves_transport_failures() {
    let mut client = Client::new(TransportFailure);
    assert!(matches!(
        client.create_cluster(create_request()),
        Err(ClientError::Transport(TransportFailure))
    ));
    assert!(matches!(
        client.join_cluster(ClusterJoinRequest {
            cluster: ClusterId::new(7).unwrap(),
            invitation: 9,
            node: NodeId::new(2).unwrap(),
        }),
        Err(ClientError::Transport(TransportFailure))
    ));
    assert!(matches!(
        client.leave_cluster(ClusterLeaveRequest {
            cluster: ClusterId::new(7).unwrap(),
            node: NodeId::new(2).unwrap(),
            force: false,
        }),
        Err(ClientError::Transport(TransportFailure))
    ));
    assert!(matches!(
        client.remove_cluster(ClusterRemoveRequest {
            cluster: ClusterId::new(7).unwrap(),
            confirmation: 1,
        }),
        Err(ClientError::Transport(TransportFailure))
    ));
}

#[test]
fn polling_rejects_zero_and_over_limit_before_transport() {
    let mut client = Client::new(TransportFailure);
    let subscription = synos_client_sdk::Subscription {
        kind: synos_client_sdk::SubscriptionKind::Lifecycle,
        cursor: 0,
    };
    assert!(matches!(
        client.poll(subscription, 0),
        Err(ClientError::Protocol(ProtocolError::InvalidValue))
    ));
    assert!(matches!(
        client.poll(subscription, synos_client_sdk::MAX_CLUSTER_CHANGES as u8 + 1),
        Err(ClientError::Protocol(ProtocolError::InvalidValue))
    ));
}
