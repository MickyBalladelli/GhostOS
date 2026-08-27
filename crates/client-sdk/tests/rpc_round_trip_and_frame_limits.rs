// Inventory: coverage_59_7.rs (legacy roadmap section 59).
use ghostos_client_sdk::{
    Client, ClientError, ClusterNode, ClusterState, FrameHeader, JobSpec, Method, NodeHealth,
    NodeId, ProtocolError, RpcStatus, RpcTransport, Rights, TransportRights,
};

struct BusyTransport;

impl RpcTransport for BusyTransport {
    type Error = ();

    fn round_trip(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, Self::Error> {
        let request_header = FrameHeader::decode(request).unwrap();
        FrameHeader {
            method: request_header.method,
            flags: 0,
            request_id: request_header.request_id,
            payload_bytes: 0,
            status: RpcStatus::Busy,
        }
        .encode(response)
        .unwrap();
        Ok(ghostos_client_sdk::FRAME_HEADER_BYTES)
    }
}

#[test]
fn rpc_frames_reject_unknown_flags_and_truncated_payloads() {
    let header = FrameHeader {
        method: Method::ClusterState,
        flags: 0,
        request_id: 9,
        payload_bytes: 3,
        status: RpcStatus::Ok,
    };
    let mut frame = [0; 32];
    header.encode(&mut frame).unwrap();
    assert_eq!(FrameHeader::decode(&frame[..24]), Err(ProtocolError::InvalidFrame));
    frame[6] = 0x80;
    assert_eq!(FrameHeader::decode(&frame), Err(ProtocolError::InvalidFrame));
}

#[test]
fn client_propagates_busy_and_model_guards_duplicates() {
    let mut client = Client::new(BusyTransport);
    assert_eq!(
        client.cluster_state(),
        Err(ClientError::Remote(ghostos_client_sdk::RemoteError::new(
            RpcStatus::Busy,
            Method::ClusterState,
            1,
        )))
    );

    let node = ClusterNode {
        node: NodeId::new(2).unwrap(),
        health: NodeHealth::Healthy,
        cpu_load_permille: 500,
        memory_used_bytes: 10,
        memory_total_bytes: 20,
        running_jobs: 1,
        queued_jobs: 0,
    };
    let mut state = ClusterState::new(3, 4);
    state.push(node).unwrap();
    assert_eq!(state.push(node), Err(ProtocolError::DuplicateNode));
    assert_eq!(state.node_count(), 1);
    assert!(JobSpec::new("", 1, 1).is_err());
    assert!(JobSpec::new("show", 0, 1).is_err());
    assert!(JobSpec::new("show", 1, 0).is_err());
}

#[test]
fn delegation_model_requires_real_resource_and_rights() {
    let subject = NodeId::new(2).unwrap();
    assert!(ghostos_client_sdk::CapabilityDelegation::new(
        0,
        subject,
        Rights::READ,
        TransportRights::LAYER2,
        10,
    )
    .is_err());
    assert!(ghostos_client_sdk::CapabilityDelegation::new(
        7,
        subject,
        Rights::READ,
        TransportRights::LAYER2,
        10,
    )
    .is_ok());
}
