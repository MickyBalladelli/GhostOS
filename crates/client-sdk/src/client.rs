use core::convert::Infallible;

use synos_auth::CryptographicCapability;
use synos_fabric::NodeId;

use crate::{
    CapabilityDelegation, ClusterNode, ClusterState, FrameHeader, JobReceipt, JobSpec, NodeHealth,
    TopologyLink, TopologyReachability, TopologyRoute, TopologyState, TopologyTransport,
    ProtocolError, RpcStatus,
    wire::{
        FLAG_CAPABILITY, FRAME_HEADER_BYTES, MAX_FRAME_BYTES, Method, read_array, read_u16,
        read_u32, read_u64, write, write_u16, write_u32, write_u64,
    },
};

const CLUSTER_HEADER_BYTES: usize = 24;
const CLUSTER_NODE_BYTES: usize = 32;
const TOPOLOGY_HEADER_BYTES: usize = 24;
const TOPOLOGY_LINK_BYTES: usize = 32;
const JOB_REQUEST_HEADER_BYTES: usize = 18;
const JOB_RECEIPT_BYTES: usize = 16;
const DELEGATION_BYTES: usize = 24;

pub trait RpcTransport {
    type Error;

    fn round_trip(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError<E> {
    Protocol(ProtocolError),
    Remote(RpcStatus),
    Transport(E),
}

impl From<ProtocolError> for ClientError<Infallible> {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

pub struct Client<T> {
    transport: T,
    authority: Option<CryptographicCapability>,
    next_request_id: u64,
}

impl<T> Client<T> {
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            authority: None,
            next_request_id: 1,
        }
    }

    pub const fn with_authority(transport: T, authority: CryptographicCapability) -> Self {
        Self {
            transport,
            authority: Some(authority),
            next_request_id: 1,
        }
    }

    pub fn set_authority(&mut self, authority: Option<CryptographicCapability>) {
        self.authority = authority
    }

    pub fn authority(&self) -> Option<CryptographicCapability> {
        self.authority
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: RpcTransport> Client<T> {
    pub fn cluster_state(&mut self) -> Result<ClusterState, ClientError<T::Error>> {
        let mut request = [0; MAX_FRAME_BYTES];
        let payload_start = self.encode_authority(&mut request)?;
        let (response, response_bytes, header) =
            self.call(Method::ClusterState, &mut request, payload_start)?;
        decode_cluster_state(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn submit_job(&mut self, job: JobSpec<'_>) -> Result<JobReceipt, ClientError<T::Error>> {
        let job = JobSpec::new(job.command, job.priority, job.timeout_us)
            .map_err(ClientError::Protocol)?;
        let mut request = [0; MAX_FRAME_BYTES];
        let payload_start = self.encode_authority(&mut request)?;
        let body = FRAME_HEADER_BYTES + payload_start;
        let payload_bytes = payload_start + JOB_REQUEST_HEADER_BYTES + job.command.len();
        request
            .get_mut(body..body + JOB_REQUEST_HEADER_BYTES)
            .ok_or(ClientError::Protocol(ProtocolError::BufferTooSmall))?
            .fill(0);
        request[body] = job.priority;
        write_u64(&mut request, body + 8, job.timeout_us).map_err(ClientError::Protocol)?;
        write_u16(&mut request, body + 16, job.command.len() as u16)
            .map_err(ClientError::Protocol)?;
        write(
            &mut request,
            body + JOB_REQUEST_HEADER_BYTES,
            job.command.as_bytes(),
        )
        .map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) =
            self.call(Method::SubmitJob, &mut request, payload_bytes)?;
        decode_job_receipt(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn topology_state(&mut self) -> Result<TopologyState, ClientError<T::Error>> {
        let mut request = [0; MAX_FRAME_BYTES];
        let payload_start = self.encode_authority(&mut request)?;
        let (response, response_bytes, header) =
            self.call(Method::TopologyState, &mut request, payload_start)?;
        decode_topology_state(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn delegate_capability(
        &mut self,
        delegation: CapabilityDelegation,
    ) -> Result<CryptographicCapability, ClientError<T::Error>> {
        let delegation = CapabilityDelegation::new(
            delegation.resource,
            delegation.subject,
            delegation.rights,
            delegation.transports,
            delegation.valid_for_us,
        )
        .map_err(ClientError::Protocol)?;
        let mut request = [0; MAX_FRAME_BYTES];
        let payload_start = self.encode_authority(&mut request)?;
        let body = FRAME_HEADER_BYTES + payload_start;
        let payload_bytes = payload_start + DELEGATION_BYTES;
        request
            .get_mut(body..body + DELEGATION_BYTES)
            .ok_or(ClientError::Protocol(ProtocolError::BufferTooSmall))?
            .fill(0);
        write_u64(&mut request, body, delegation.resource).map_err(ClientError::Protocol)?;
        write_u32(&mut request, body + 8, delegation.subject.raw())
            .map_err(ClientError::Protocol)?;
        write_u16(&mut request, body + 12, delegation.rights.bits())
            .map_err(ClientError::Protocol)?;
        request[body + 14] = delegation.transports.bits();
        write_u64(&mut request, body + 16, delegation.valid_for_us)
            .map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) =
            self.call(Method::DelegateCapability, &mut request, payload_bytes)?;
        decode_capability(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    fn encode_authority(
        &self,
        request: &mut [u8; MAX_FRAME_BYTES],
    ) -> Result<usize, ClientError<T::Error>> {
        let Some(authority) = self.authority else {
            return Ok(0);
        };
        write(request, FRAME_HEADER_BYTES, &authority.encode()).map_err(ClientError::Protocol)?;
        Ok(CryptographicCapability::WIRE_BYTES)
    }

    fn call(
        &mut self,
        method: Method,
        request: &mut [u8; MAX_FRAME_BYTES],
        payload_bytes: usize,
    ) -> Result<([u8; MAX_FRAME_BYTES], usize, FrameHeader), ClientError<T::Error>> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let header = FrameHeader {
            method,
            flags: if self.authority.is_some() {
                FLAG_CAPABILITY
            } else {
                0
            },
            request_id,
            payload_bytes: payload_bytes as u32,
            status: RpcStatus::Ok,
        };
        header.encode(request).map_err(ClientError::Protocol)?;
        let request_bytes = FRAME_HEADER_BYTES + payload_bytes;
        let mut response = [0; MAX_FRAME_BYTES];
        let response_bytes = self
            .transport
            .round_trip(&request[..request_bytes], &mut response)
            .map_err(ClientError::Transport)?;
        if response_bytes > response.len() {
            return Err(ClientError::Protocol(ProtocolError::InvalidFrame));
        }
        let response_header =
            FrameHeader::decode(&response[..response_bytes]).map_err(ClientError::Protocol)?;
        if response_header.request_id != request_id
            || response_header.method != method
            || response_header.flags != 0
            || response_bytes != FRAME_HEADER_BYTES + response_header.payload_bytes as usize
        {
            return Err(ClientError::Protocol(ProtocolError::MismatchedResponse));
        }
        if response_header.status != RpcStatus::Ok {
            return Err(ClientError::Remote(response_header.status));
        }
        Ok((response, response_bytes, response_header))
    }
}

pub(crate) fn encode_cluster_state(
    state: ClusterState,
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    let required = CLUSTER_HEADER_BYTES + state.node_count() * CLUSTER_NODE_BYTES;
    if output.len() < required {
        return Err(ProtocolError::BufferTooSmall);
    }
    output[..required].fill(0);
    write_u64(output, 0, state.generation)?;
    write_u64(output, 8, state.sampled_at_us)?;
    write_u16(output, 16, state.node_count() as u16)?;
    for (index, node) in state.nodes().enumerate() {
        let offset = CLUSTER_HEADER_BYTES + index * CLUSTER_NODE_BYTES;
        write_u32(output, offset, node.node.raw())?;
        output[offset + 4] = node.health as u8;
        write_u16(output, offset + 6, node.cpu_load_permille)?;
        write_u64(output, offset + 8, node.memory_used_bytes)?;
        write_u64(output, offset + 16, node.memory_total_bytes)?;
        write_u32(output, offset + 24, node.running_jobs)?;
        write_u32(output, offset + 28, node.queued_jobs)?;
    }
    Ok(required)
}

fn decode_cluster_state(frame: &[u8], header: FrameHeader) -> Result<ClusterState, ProtocolError> {
    let payload = frame
        .get(FRAME_HEADER_BYTES..)
        .ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < CLUSTER_HEADER_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_CLUSTER_NODES
        || payload.len() != CLUSTER_HEADER_BYTES + count * CLUSTER_NODE_BYTES
        || header.payload_bytes as usize != payload.len()
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut state = ClusterState::new(read_u64(payload, 0)?, read_u64(payload, 8)?);
    for index in 0..count {
        let offset = CLUSTER_HEADER_BYTES + index * CLUSTER_NODE_BYTES;
        state.push(ClusterNode {
            node: NodeId::new(read_u32(payload, offset)?).ok_or(ProtocolError::InvalidValue)?,
            health: NodeHealth::from_wire(payload[offset + 4])?,
            cpu_load_permille: read_u16(payload, offset + 6)?,
            memory_used_bytes: read_u64(payload, offset + 8)?,
            memory_total_bytes: read_u64(payload, offset + 16)?,
            running_jobs: read_u32(payload, offset + 24)?,
            queued_jobs: read_u32(payload, offset + 28)?,
        })?;
    }
    Ok(state)
}

pub(crate) fn encode_topology_state(
    state: TopologyState,
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    let required = TOPOLOGY_HEADER_BYTES + state.link_count() * TOPOLOGY_LINK_BYTES;
    if output.len() < required {
        return Err(ProtocolError::BufferTooSmall)
    }
    output[..required].fill(0);
    write_u64(output, 0, state.generation)?;
    write_u64(output, 8, state.sampled_at_us)?;
    write_u16(output, 16, state.link_count() as u16)?;
    for (index, link) in state.links().enumerate() {
        let offset = TOPOLOGY_HEADER_BYTES + index * TOPOLOGY_LINK_BYTES;
        write_u32(output, offset, link.from.raw())?;
        write_u32(output, offset + 4, link.to.raw())?;
        output[offset + 8] = link.transport as u8;
        output[offset + 9] = link.route as u8;
        output[offset + 10] = link.reachability as u8;
        write_u64(output, offset + 16, link.latency_us)?;
        write_u64(output, offset + 24, link.bandwidth_mbps)?;
        write_u16(output, offset + 12, link.mtu)?;
    }
    Ok(required)
}

fn decode_topology_state(frame: &[u8], header: FrameHeader) -> Result<TopologyState, ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < TOPOLOGY_HEADER_BYTES {
        return Err(ProtocolError::InvalidFrame)
    }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_TOPOLOGY_LINKS
        || payload.len() != TOPOLOGY_HEADER_BYTES + count * TOPOLOGY_LINK_BYTES
        || header.payload_bytes as usize != payload.len()
    {
        return Err(ProtocolError::InvalidFrame)
    }
    let mut state = TopologyState::new(read_u64(payload, 0)?, read_u64(payload, 8)?);
    for index in 0..count {
        let offset = TOPOLOGY_HEADER_BYTES + index * TOPOLOGY_LINK_BYTES;
        state.push(TopologyLink {
            from: NodeId::new(read_u32(payload, offset)?).ok_or(ProtocolError::InvalidValue)?,
            to: NodeId::new(read_u32(payload, offset + 4)?).ok_or(ProtocolError::InvalidValue)?,
            transport: TopologyTransport::from_wire(payload[offset + 8])?,
            route: TopologyRoute::from_wire(payload[offset + 9])?,
            reachability: TopologyReachability::from_wire(payload[offset + 10])?,
            mtu: read_u16(payload, offset + 12)?,
            latency_us: read_u64(payload, offset + 16)?,
            bandwidth_mbps: read_u64(payload, offset + 24)?,
        })?;
    }
    Ok(state)
}

pub(crate) fn decode_job_spec(input: &[u8]) -> Result<JobSpec<'_>, ProtocolError> {
    if input.len() < JOB_REQUEST_HEADER_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let command_bytes = read_u16(input, 16)? as usize;
    if input.len() != JOB_REQUEST_HEADER_BYTES + command_bytes {
        return Err(ProtocolError::InvalidFrame);
    }
    let command = core::str::from_utf8(&input[JOB_REQUEST_HEADER_BYTES..])
        .map_err(|_| ProtocolError::InvalidUtf8)?;
    JobSpec::new(command, input[0], read_u64(input, 8)?)
}

pub(crate) fn encode_job_receipt(
    receipt: JobReceipt,
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    let receipt = receipt.validate()?;
    if output.len() < JOB_RECEIPT_BYTES {
        return Err(ProtocolError::BufferTooSmall);
    }
    output[..JOB_RECEIPT_BYTES].fill(0);
    write_u64(output, 0, receipt.job_id)?;
    write_u64(output, 8, receipt.accepted_at_us)?;
    Ok(JOB_RECEIPT_BYTES)
}

fn decode_job_receipt(frame: &[u8], header: FrameHeader) -> Result<JobReceipt, ProtocolError> {
    if header.payload_bytes as usize != JOB_RECEIPT_BYTES
        || frame.len() != FRAME_HEADER_BYTES + JOB_RECEIPT_BYTES
    {
        return Err(ProtocolError::InvalidFrame);
    }
    JobReceipt {
        job_id: read_u64(frame, FRAME_HEADER_BYTES)?,
        accepted_at_us: read_u64(frame, FRAME_HEADER_BYTES + 8)?,
    }
    .validate()
}

pub(crate) fn decode_delegation(input: &[u8]) -> Result<CapabilityDelegation, ProtocolError> {
    if input.len() != DELEGATION_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    CapabilityDelegation::new(
        read_u64(input, 0)?,
        NodeId::new(read_u32(input, 8)?).ok_or(ProtocolError::InvalidValue)?,
        synos_kernel::Rights::from_bits(read_u16(input, 12)?).ok_or(ProtocolError::InvalidValue)?,
        synos_auth::TransportRights::from_bits(input[14]).ok_or(ProtocolError::InvalidValue)?,
        read_u64(input, 16)?,
    )
}

pub(crate) fn encode_capability(
    capability: CryptographicCapability,
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    write(output, 0, &capability.encode())?;
    Ok(CryptographicCapability::WIRE_BYTES)
}

fn decode_capability(
    frame: &[u8],
    header: FrameHeader,
) -> Result<CryptographicCapability, ProtocolError> {
    if header.payload_bytes as usize != CryptographicCapability::WIRE_BYTES
        || frame.len() != FRAME_HEADER_BYTES + CryptographicCapability::WIRE_BYTES
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let bytes = read_array::<{ CryptographicCapability::WIRE_BYTES }>(frame, FRAME_HEADER_BYTES)?;
    CryptographicCapability::decode(bytes).map_err(|_| ProtocolError::InvalidValue)
}
