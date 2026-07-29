use synos_auth::CryptographicCapability;

use crate::{
    CapabilityDelegation, ClusterState, FrameHeader, JobReceipt, JobSpec, Method, ProtocolError,
    RpcStatus,
    client::{
        decode_delegation, decode_job_spec, encode_capability, encode_cluster_state,
        encode_job_receipt,
    },
    wire::{FLAG_CAPABILITY, FRAME_HEADER_BYTES, read_array},
};

pub trait GatewayService {
    fn cluster_state(
        &mut self,
        authority: Option<CryptographicCapability>,
    ) -> Result<ClusterState, RpcStatus>;

    fn submit_job(
        &mut self,
        authority: Option<CryptographicCapability>,
        job: JobSpec<'_>,
    ) -> Result<JobReceipt, RpcStatus>;

    fn delegate_capability(
        &mut self,
        authority: Option<CryptographicCapability>,
        delegation: CapabilityDelegation,
    ) -> Result<CryptographicCapability, RpcStatus>;
}

pub struct FrontendGateway<S> {
    service: S,
}

impl<S> FrontendGateway<S> {
    pub const fn new(service: S) -> Self {
        Self { service }
    }

    pub fn service(&self) -> &S {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut S {
        &mut self.service
    }

    pub fn into_service(self) -> S {
        self.service
    }
}

impl<S: GatewayService> FrontendGateway<S> {
    pub fn handle(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, ProtocolError> {
        if response.len() < FRAME_HEADER_BYTES {
            return Err(ProtocolError::BufferTooSmall);
        }
        let request_header = FrameHeader::decode(request)?;
        if request.len() != FRAME_HEADER_BYTES + request_header.payload_bytes as usize
            || request_header.status != RpcStatus::Ok
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let payload = &request[FRAME_HEADER_BYTES..];
        let (authority, body) = decode_authority(request_header.flags, payload)?;
        let outcome = match request_header.method {
            Method::ClusterState => {
                if !body.is_empty() {
                    Err(RpcStatus::InvalidRequest)
                } else {
                    self.service.cluster_state(authority).and_then(|state| {
                        encode_cluster_state(state, &mut response[FRAME_HEADER_BYTES..])
                            .map_err(|_| RpcStatus::Internal)
                    })
                }
            }
            Method::SubmitJob => decode_job_spec(body)
                .map_err(|_| RpcStatus::InvalidRequest)
                .and_then(|job| self.service.submit_job(authority, job))
                .and_then(|receipt| {
                    encode_job_receipt(receipt, &mut response[FRAME_HEADER_BYTES..])
                        .map_err(|_| RpcStatus::Internal)
                }),
            Method::DelegateCapability => decode_delegation(body)
                .map_err(|_| RpcStatus::InvalidRequest)
                .and_then(|delegation| self.service.delegate_capability(authority, delegation))
                .and_then(|capability| {
                    encode_capability(capability, &mut response[FRAME_HEADER_BYTES..])
                        .map_err(|_| RpcStatus::Internal)
                }),
        };
        let (status, payload_bytes) = match outcome {
            Ok(payload_bytes) => (RpcStatus::Ok, payload_bytes),
            Err(status) => (status, 0),
        };
        FrameHeader {
            method: request_header.method,
            flags: 0,
            request_id: request_header.request_id,
            payload_bytes: payload_bytes as u32,
            status,
        }
        .encode(response)?;
        Ok(FRAME_HEADER_BYTES + payload_bytes)
    }
}

fn decode_authority(
    flags: u16,
    payload: &[u8],
) -> Result<(Option<CryptographicCapability>, &[u8]), ProtocolError> {
    if flags & FLAG_CAPABILITY == 0 {
        return Ok((None, payload));
    }
    if payload.len() < CryptographicCapability::WIRE_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let bytes = read_array::<{ CryptographicCapability::WIRE_BYTES }>(payload, 0)?;
    let authority =
        CryptographicCapability::decode(bytes).map_err(|_| ProtocolError::InvalidValue)?;
    Ok((
        Some(authority),
        &payload[CryptographicCapability::WIRE_BYTES..],
    ))
}
