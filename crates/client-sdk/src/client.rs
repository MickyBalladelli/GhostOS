use core::convert::Infallible;

use ghostos_auth::CryptographicCapability;
use ghostos_fabric::NodeId;
use ghostos_ipc::{BufferError, BufferLease, BufferOwner, BufferRights};
use ghostos_observability::{ProfileDomain, ProfileSample, record_profile_sample};
use ghostos_status::PublicError;
use ghostos_system_model::performance::PerformanceDiagnostics;

use crate::{
    AuditEventList, CapabilityDelegation, ChangeBatch, ClusterCreateRequest, ClusterHealthSnapshot,
    ClusterId, ClusterJoinRequest, ClusterLeaveRequest, ClusterMember, ClusterNode,
    ClusterRemoveRequest,
    ClusterResources, ClusterState, ClusterSummary, FrameHeader, InvitationList,
    JobReceipt, JobSpec, JoinPlan, LeavePlan, LifecycleReceipt, MemberList, NodeHealth,
    Subscription, SubscriptionKind, TopologyLink, TopologyReachability, TopologyRoute,
    TopologyState, TopologyTransport, ProtocolError, RpcStatus,
    wire::{
        FLAG_CAPABILITY, FRAME_HEADER_BYTES, MAX_FRAME_BYTES, Method, read_array, read_u16,
        read_u32, read_u64, write, write_u16, write_u32, write_u64,
        decode_performance_diagnostics, PERFORMANCE_DIAGNOSTICS_BYTES,
    },
};

const CLUSTER_HEADER_BYTES: usize = 24;
const CLUSTER_NODE_BYTES: usize = 32;
const TOPOLOGY_HEADER_BYTES: usize = 24;
const TOPOLOGY_LINK_BYTES: usize = 32;
const JOB_REQUEST_HEADER_BYTES: usize = 18;
const JOB_RECEIPT_BYTES: usize = 16;
const DELEGATION_BYTES: usize = 24;
const SUMMARY_BYTES: usize = 128;
const MEMBER_HEADER_BYTES: usize = 24;
const MEMBER_BYTES: usize = 24;
const INVITATION_HEADER_BYTES: usize = 24;
const INVITATION_BYTES: usize = 32;
const JOIN_PLAN_BYTES: usize = 32;
const LEAVE_PLAN_BYTES: usize = 32;
const HEALTH_BYTES: usize = 48;
const RESOURCE_BYTES: usize = 96;
const AUDIT_HEADER_BYTES: usize = 24;
const AUDIT_EVENT_BYTES: usize = 48;
const LIFECYCLE_RECEIPT_BYTES: usize = 24;
const SUBSCRIPTION_BYTES: usize = 16;
const CHANGE_HEADER_BYTES: usize = 24;
const CHANGE_EVENT_BYTES: usize = 32;
const CLUSTER_CREATE_BYTES: usize = 88;
const CLUSTER_JOIN_BYTES: usize = 32;
const CLUSTER_LEAVE_BYTES: usize = 32;
const CLUSTER_REMOVE_BYTES: usize = 24;
const POLL_BYTES: usize = 16;

pub trait RpcTransport {
    type Error;

    fn round_trip(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, Self::Error>;

    fn round_trip_loaned(
        &mut self,
        request: &mut BufferLease<'_>,
        response: &mut BufferLease<'_>,
    ) -> Result<usize, LoanedRpcError<Self::Error>> {
        if request.owner() != BufferOwner::RpcClient
            || response.owner() != BufferOwner::RpcClient
        {
            return Err(LoanedRpcError::Buffer(BufferError::OwnerMismatch));
        }
        if !request
            .capability()
            .rights()
            .contains(BufferRights::READ.union(BufferRights::TRANSFER))
            || !response
                .capability()
                .rights()
                .contains(BufferRights::WRITE.union(BufferRights::TRANSFER))
        {
            return Err(LoanedRpcError::Buffer(BufferError::CapabilityDenied));
        }
        request
            .transfer_to(BufferOwner::RpcTransport)
            .map_err(LoanedRpcError::Buffer)?;
        response
            .transfer_to(BufferOwner::RpcTransport)
            .map_err(LoanedRpcError::Buffer)?;
        let result = response
            .as_mut_slice()
            .map_err(LoanedRpcError::Buffer)
            .and_then(|response_bytes| {
                let request_bytes = request.as_slice().map_err(LoanedRpcError::Buffer)?;
                self.round_trip(request_bytes, response_bytes)
                    .map_err(LoanedRpcError::Transport)
            });
        request
            .transfer_to(BufferOwner::RpcClient)
            .map_err(LoanedRpcError::Buffer)?;
        response
            .transfer_to(BufferOwner::RpcClient)
            .map_err(LoanedRpcError::Buffer)?;
        result
    }
}

#[derive(Debug)]
pub enum LoanedRpcError<E> {
    Buffer(BufferError),
    Transport(E),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError<E> {
    Protocol(ProtocolError),
    Remote(RemoteError),
    Transport(E),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteError {
    pub status: RpcStatus,
    pub error: PublicError,
}

impl RemoteError {
    pub const fn new(status: RpcStatus, operation: Method, request_id: u64) -> Self {
        Self {
            status,
            error: status.public_status().public_error(
                operation as u16,
                ghostos_status::AuditContext::new(request_id as u128, 0),
            ),
        }
    }
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
    last_diagnostics: PerformanceDiagnostics,
}

impl<T> Client<T> {
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            authority: None,
            next_request_id: 1,
            last_diagnostics: PerformanceDiagnostics::empty(),
        }
    }

    pub const fn with_authority(transport: T, authority: CryptographicCapability) -> Self {
        Self {
            transport,
            authority: Some(authority),
            next_request_id: 1,
            last_diagnostics: PerformanceDiagnostics::empty(),
        }
    }

    pub fn set_authority(&mut self, authority: Option<CryptographicCapability>) {
        self.authority = authority
    }

    pub fn authority(&self) -> Option<CryptographicCapability> {
        self.authority
    }

    /// Diagnostics contain timing, retry, and byte counters only. They never
    /// contain command arguments, tenant identifiers, or response payloads.
    pub const fn last_diagnostics(&self) -> PerformanceDiagnostics {
        self.last_diagnostics
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

    pub fn cluster_summary(&mut self) -> Result<ClusterSummary, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterSummary)?;
        decode_cluster_summary(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_members(&mut self) -> Result<MemberList, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterMembers)?;
        decode_member_list(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_invitations(&mut self) -> Result<InvitationList, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterInvitations)?;
        decode_invitation_list(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_join_plan(&mut self) -> Result<JoinPlan, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterJoinPlan)?;
        decode_join_plan(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_leave_plan(&mut self) -> Result<LeavePlan, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterLeavePlan)?;
        decode_leave_plan(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_health(&mut self) -> Result<ClusterHealthSnapshot, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterHealth)?;
        decode_cluster_health(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_resources(&mut self) -> Result<ClusterResources, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterResources)?;
        decode_cluster_resources(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn cluster_audit(&mut self) -> Result<AuditEventList, ClientError<T::Error>> {
        let (response, response_bytes, header) = self.call_empty(Method::ClusterAudit)?;
        decode_audit_events(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn create_cluster(
        &mut self,
        request: ClusterCreateRequest,
    ) -> Result<LifecycleReceipt, ClientError<T::Error>> {
        let mut body = [0; CLUSTER_CREATE_BYTES];
        encode_cluster_create(request, &mut body).map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) = self.call_body(Method::ClusterCreate, &body)?;
        decode_lifecycle_receipt(&response[..response_bytes], header)
            .map_err(ClientError::Protocol)
    }

    pub fn join_cluster(
        &mut self,
        request: ClusterJoinRequest,
    ) -> Result<LifecycleReceipt, ClientError<T::Error>> {
        let mut body = [0; CLUSTER_JOIN_BYTES];
        encode_cluster_join(request, &mut body).map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) = self.call_body(Method::ClusterJoin, &body)?;
        decode_lifecycle_receipt(&response[..response_bytes], header)
            .map_err(ClientError::Protocol)
    }

    pub fn leave_cluster(
        &mut self,
        request: ClusterLeaveRequest,
    ) -> Result<LifecycleReceipt, ClientError<T::Error>> {
        let mut body = [0; CLUSTER_LEAVE_BYTES];
        encode_cluster_leave(request, &mut body).map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) = self.call_body(Method::ClusterLeave, &body)?;
        decode_lifecycle_receipt(&response[..response_bytes], header)
            .map_err(ClientError::Protocol)
    }

    pub fn remove_cluster(
        &mut self,
        request: ClusterRemoveRequest,
    ) -> Result<LifecycleReceipt, ClientError<T::Error>> {
        let mut body = [0; CLUSTER_REMOVE_BYTES];
        encode_cluster_remove(request, &mut body).map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) = self.call_body(Method::ClusterRemove, &body)?;
        decode_lifecycle_receipt(&response[..response_bytes], header)
            .map_err(ClientError::Protocol)
    }

    pub fn subscribe(
        &mut self,
        kind: SubscriptionKind,
    ) -> Result<Subscription, ClientError<T::Error>> {
        let body = [kind as u8, 0, 0, 0, 0, 0, 0, 0];
        let (response, response_bytes, header) = self.call_body(Method::Subscribe, &body)?;
        decode_subscription(&response[..response_bytes], header).map_err(ClientError::Protocol)
    }

    pub fn poll(
        &mut self,
        subscription: Subscription,
        limit: u8,
    ) -> Result<ChangeBatch, ClientError<T::Error>> {
        if limit == 0 || limit as usize > crate::MAX_CLUSTER_CHANGES {
            return Err(ClientError::Protocol(ProtocolError::InvalidValue));
        }
        let mut body = [0; POLL_BYTES];
        body[0] = subscription.kind as u8;
        body[1] = limit;
        write_u64(&mut body, 8, subscription.cursor).map_err(ClientError::Protocol)?;
        let (response, response_bytes, header) = self.call_body(Method::Poll, &body)?;
        decode_change_batch(&response[..response_bytes], header).map_err(ClientError::Protocol)
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

    fn call_empty(
        &mut self,
        method: Method,
    ) -> Result<([u8; MAX_FRAME_BYTES], usize, FrameHeader), ClientError<T::Error>> {
        let mut request = [0; MAX_FRAME_BYTES];
        let payload_start = self.encode_authority(&mut request)?;
        self.call(method, &mut request, payload_start)
    }

    fn call_body(
        &mut self,
        method: Method,
        body: &[u8],
    ) -> Result<([u8; MAX_FRAME_BYTES], usize, FrameHeader), ClientError<T::Error>> {
        let mut request = [0; MAX_FRAME_BYTES];
        let authority_bytes = self.encode_authority(&mut request)?;
        let offset = FRAME_HEADER_BYTES + authority_bytes;
        write(&mut request, offset, body).map_err(ClientError::Protocol)?;
        self.call(method, &mut request, authority_bytes + body.len())
    }

    fn call(
        &mut self,
        method: Method,
        request: &mut [u8; MAX_FRAME_BYTES],
        payload_bytes: usize,
    ) -> Result<([u8; MAX_FRAME_BYTES], usize, FrameHeader), ClientError<T::Error>> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        record_profile_sample(ProfileSample::single(
            ProfileDomain::ClientRpc,
            request_id,
            0,
            0x9001 + method as u64,
        ));
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
        let response_payload_bytes = response_header.payload_bytes as usize;
        if response_header.status != RpcStatus::Ok {
            if response_payload_bytes >= PERFORMANCE_DIAGNOSTICS_BYTES {
                let start = FRAME_HEADER_BYTES + response_payload_bytes
                    - PERFORMANCE_DIAGNOSTICS_BYTES;
                self.last_diagnostics = decode_performance_diagnostics(
                    &response[start..FRAME_HEADER_BYTES + response_payload_bytes],
                )
                .map_err(ClientError::Protocol)?;
            } else {
                self.last_diagnostics = PerformanceDiagnostics::empty();
            }
            return Err(ClientError::Remote(RemoteError::new(
                response_header.status,
                method,
                request_id,
            )));
        }
        if response_payload_bytes < PERFORMANCE_DIAGNOSTICS_BYTES {
            return Err(ClientError::Protocol(ProtocolError::InvalidFrame))
        }
        let result_payload_bytes = response_payload_bytes - PERFORMANCE_DIAGNOSTICS_BYTES;
        let diagnostics_start = FRAME_HEADER_BYTES + result_payload_bytes;
        self.last_diagnostics = decode_performance_diagnostics(
            &response[diagnostics_start..FRAME_HEADER_BYTES + response_payload_bytes],
        )
        .map_err(ClientError::Protocol)?;
        let mut result_header = response_header;
        result_header.payload_bytes = result_payload_bytes as u32;
        Ok((response, FRAME_HEADER_BYTES + result_payload_bytes, result_header))
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
        ghostos_kernel::Rights::from_bits(read_u16(input, 12)?).ok_or(ProtocolError::InvalidValue)?,
        ghostos_auth::TransportRights::from_bits(input[14]).ok_or(ProtocolError::InvalidValue)?,
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

fn cluster_payload<'a>(
    frame: &'a [u8],
    header: FrameHeader,
    required: usize,
) -> Result<&'a [u8], ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() != required || header.payload_bytes as usize != required {
        return Err(ProtocolError::InvalidFrame);
    }
    Ok(payload)
}

fn read_u128(input: &[u8], offset: usize) -> Result<u128, ProtocolError> {
    Ok(u128::from_be_bytes(read_array::<16>(input, offset)?))
}

fn write_u128(output: &mut [u8], offset: usize, value: u128) -> Result<(), ProtocolError> {
    write(output, offset, &value.to_be_bytes())
}

pub(crate) fn encode_cluster_summary(
    summary: ClusterSummary,
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    if output.len() < SUMMARY_BYTES {
        return Err(ProtocolError::BufferTooSmall);
    }
    output[..SUMMARY_BYTES].fill(0);
    write_u128(output, 0, summary.cluster.raw())?;
    output[16] = summary.lifecycle as u8;
    output[17] = summary.health as u8;
    write_u64(output, 20, summary.generation)?;
    write_u64(output, 28, summary.sampled_at_us)?;
    write_u32(output, 36, summary.leader.raw())?;
    write_u32(output, 40, summary.coordinator.raw())?;
    write_u16(output, 44, summary.member_count)?;
    write_u16(output, 46, summary.healthy_members)?;
    write_u16(output, 48, summary.voting_members)?;
    write_u16(output, 50, summary.quorum_required)?;
    write_u16(output, 52, summary.quorum_available)?;
    write(output, 56, &summary.name.as_bytes())?;
    Ok(SUMMARY_BYTES)
}

fn decode_cluster_summary(
    frame: &[u8],
    header: FrameHeader,
) -> Result<ClusterSummary, ProtocolError> {
    let payload = cluster_payload(frame, header, SUMMARY_BYTES)?;
    Ok(ClusterSummary {
        cluster: ClusterId::new(read_u128(payload, 0)?).ok_or(ProtocolError::InvalidValue)?,
        name: crate::ClusterName::from_wire(&payload[56..56 + crate::MAX_CLUSTER_NAME_BYTES])?,
        lifecycle: crate::ClusterLifecycle::from_wire(payload[16])?,
        health: crate::ClusterHealth::from_wire(payload[17])?,
        generation: read_u64(payload, 20)?,
        sampled_at_us: read_u64(payload, 28)?,
        leader: NodeId::new(read_u32(payload, 36)?).ok_or(ProtocolError::InvalidValue)?,
        coordinator: NodeId::new(read_u32(payload, 40)?).ok_or(ProtocolError::InvalidValue)?,
        member_count: read_u16(payload, 44)?,
        healthy_members: read_u16(payload, 46)?,
        voting_members: read_u16(payload, 48)?,
        quorum_required: read_u16(payload, 50)?,
        quorum_available: read_u16(payload, 52)?,
    })
}

pub(crate) fn encode_member_list(list: MemberList, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let required = MEMBER_HEADER_BYTES + list.member_count() * MEMBER_BYTES;
    if output.len() < required {
        return Err(ProtocolError::BufferTooSmall);
    }
    output[..required].fill(0);
    write_u64(output, 0, list.generation)?;
    write_u16(output, 16, list.member_count() as u16)?;
    for (index, member) in list.members().enumerate() {
        let offset = MEMBER_HEADER_BYTES + index * MEMBER_BYTES;
        write_u32(output, offset, member.node.raw())?;
        output[offset + 4] = member.state as u8;
        output[offset + 5] = member.role as u8;
        output[offset + 6] = member.health as u8;
        write_u16(output, offset + 8, member.cpu_load_permille)?;
        write_u64(output, offset + 16, member.last_seen_us)?;
    }
    Ok(required)
}

fn decode_member_list(frame: &[u8], header: FrameHeader) -> Result<MemberList, ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < MEMBER_HEADER_BYTES || header.payload_bytes as usize != payload.len() {
        return Err(ProtocolError::InvalidFrame);
    }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_CLUSTER_MEMBERS
        || payload.len() != MEMBER_HEADER_BYTES + count * MEMBER_BYTES
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut list = MemberList::new(read_u64(payload, 0)?);
    for index in 0..count {
        let offset = MEMBER_HEADER_BYTES + index * MEMBER_BYTES;
        list.push(ClusterMember {
            node: NodeId::new(read_u32(payload, offset)?).ok_or(ProtocolError::InvalidValue)?,
            state: crate::MemberState::from_wire(payload[offset + 4])?,
            role: crate::MemberRole::from_wire(payload[offset + 5])?,
            health: NodeHealth::from_wire(payload[offset + 6])?,
            cpu_load_permille: read_u16(payload, offset + 8)?,
            last_seen_us: read_u64(payload, offset + 16)?,
        })?;
    }
    Ok(list)
}

pub(crate) fn encode_invitation_list(list: InvitationList, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let required = INVITATION_HEADER_BYTES + list.invitations().count() * INVITATION_BYTES;
    if output.len() < required {
        return Err(ProtocolError::BufferTooSmall);
    }
    output[..required].fill(0);
    write_u64(output, 0, list.generation)?;
    write_u16(output, 16, list.invitations().count() as u16)?;
    for (index, invitation) in list.invitations().enumerate() {
        let offset = INVITATION_HEADER_BYTES + index * INVITATION_BYTES;
        write_u64(output, offset, invitation.invitation)?;
        write_u32(output, offset + 8, invitation.node.raw())?;
        output[offset + 12] = invitation.state as u8;
        write_u64(output, offset + 16, invitation.issued_at_us)?;
        write_u64(output, offset + 24, invitation.expires_at_us)?;
        write_u32(output, offset + 28, invitation.scope)?;
    }
    Ok(required)
}

fn decode_invitation_list(frame: &[u8], header: FrameHeader) -> Result<InvitationList, ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < INVITATION_HEADER_BYTES || header.payload_bytes as usize != payload.len() {
        return Err(ProtocolError::InvalidFrame);
    }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_CLUSTER_INVITATIONS
        || payload.len() != INVITATION_HEADER_BYTES + count * INVITATION_BYTES
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut list = InvitationList::new(read_u64(payload, 0)?);
    for index in 0..count {
        let offset = INVITATION_HEADER_BYTES + index * INVITATION_BYTES;
        list.push(crate::ClusterInvitation {
            invitation: read_u64(payload, offset)?,
            node: NodeId::new(read_u32(payload, offset + 8)?).ok_or(ProtocolError::InvalidValue)?,
            state: crate::InvitationState::from_wire(payload[offset + 12])?,
            issued_at_us: read_u64(payload, offset + 16)?,
            expires_at_us: read_u64(payload, offset + 24)?,
            scope: read_u32(payload, offset + 28)?,
        })?;
    }
    Ok(list)
}

pub(crate) fn encode_join_plan(plan: JoinPlan, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < JOIN_PLAN_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..JOIN_PLAN_BYTES].fill(0);
    write_u64(output, 0, plan.generation)?;
    write_u64(output, 8, plan.invitation)?;
    write_u32(output, 16, plan.node.raw())?;
    output[20] = plan.requires_approval as u8;
    output[21] = plan.steps;
    write_u64(output, 24, plan.expires_at_us)?;
    Ok(JOIN_PLAN_BYTES)
}

fn decode_join_plan(frame: &[u8], header: FrameHeader) -> Result<JoinPlan, ProtocolError> {
    let payload = cluster_payload(frame, header, JOIN_PLAN_BYTES)?;
    Ok(JoinPlan {
        generation: read_u64(payload, 0)?,
        invitation: read_u64(payload, 8)?,
        node: NodeId::new(read_u32(payload, 16)?).ok_or(ProtocolError::InvalidValue)?,
        requires_approval: payload[20] != 0,
        steps: payload[21],
        expires_at_us: read_u64(payload, 24)?,
    })
}

pub(crate) fn encode_leave_plan(plan: LeavePlan, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < LEAVE_PLAN_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..LEAVE_PLAN_BYTES].fill(0);
    write_u64(output, 0, plan.generation)?;
    write_u32(output, 8, plan.node.raw())?;
    write_u32(output, 12, plan.workload_count)?;
    write_u32(output, 16, plan.lease_count)?;
    output[20] = plan.drain_required as u8;
    output[21] = plan.force_allowed as u8;
    Ok(LEAVE_PLAN_BYTES)
}

fn decode_leave_plan(frame: &[u8], header: FrameHeader) -> Result<LeavePlan, ProtocolError> {
    let payload = cluster_payload(frame, header, LEAVE_PLAN_BYTES)?;
    Ok(LeavePlan {
        generation: read_u64(payload, 0)?,
        node: NodeId::new(read_u32(payload, 8)?).ok_or(ProtocolError::InvalidValue)?,
        workload_count: read_u32(payload, 12)?,
        lease_count: read_u32(payload, 16)?,
        drain_required: payload[20] != 0,
        force_allowed: payload[21] != 0,
    })
}

pub(crate) fn encode_cluster_health(health: ClusterHealthSnapshot, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < HEALTH_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..HEALTH_BYTES].fill(0);
    write_u64(output, 0, health.generation)?;
    output[8] = health.health as u8;
    output[9] = health.quorum as u8;
    write_u64(output, 16, health.heartbeat_period_us)?;
    write_u16(output, 24, health.missed_heartbeat_limit)?;
    write_u64(output, 28, health.last_change_us)?;
    write_u16(output, 36, health.healthy_nodes)?;
    write_u16(output, 38, health.degraded_nodes)?;
    write_u16(output, 40, health.failed_nodes)?;
    Ok(HEALTH_BYTES)
}

fn decode_cluster_health(frame: &[u8], header: FrameHeader) -> Result<ClusterHealthSnapshot, ProtocolError> {
    let payload = cluster_payload(frame, header, HEALTH_BYTES)?;
    Ok(ClusterHealthSnapshot {
        generation: read_u64(payload, 0)?,
        health: crate::ClusterHealth::from_wire(payload[8])?,
        quorum: payload[9] != 0,
        heartbeat_period_us: read_u64(payload, 16)?,
        missed_heartbeat_limit: read_u16(payload, 24)?,
        last_change_us: read_u64(payload, 28)?,
        healthy_nodes: read_u16(payload, 36)?,
        degraded_nodes: read_u16(payload, 38)?,
        failed_nodes: read_u16(payload, 40)?,
    })
}

pub(crate) fn encode_cluster_resources(resources: ClusterResources, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < RESOURCE_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..RESOURCE_BYTES].fill(0);
    for (index, value) in [
        resources.generation,
        resources.cpu_capacity,
        resources.cpu_available,
        resources.memory_capacity_bytes,
        resources.memory_available_bytes,
        resources.cxl_capacity_bytes,
        resources.cxl_available_bytes,
        resources.storage_capacity_bytes,
        resources.storage_available_bytes,
        resources.network_bandwidth_mbps,
        resources.accelerator_capacity,
        resources.accelerator_available,
    ]
    .into_iter()
    .enumerate()
    {
        write_u64(output, index * 8, value)?;
    }
    Ok(RESOURCE_BYTES)
}

fn decode_cluster_resources(frame: &[u8], header: FrameHeader) -> Result<ClusterResources, ProtocolError> {
    let payload = cluster_payload(frame, header, RESOURCE_BYTES)?;
    Ok(ClusterResources {
        generation: read_u64(payload, 0)?,
        cpu_capacity: read_u64(payload, 8)?,
        cpu_available: read_u64(payload, 16)?,
        memory_capacity_bytes: read_u64(payload, 24)?,
        memory_available_bytes: read_u64(payload, 32)?,
        cxl_capacity_bytes: read_u64(payload, 40)?,
        cxl_available_bytes: read_u64(payload, 48)?,
        storage_capacity_bytes: read_u64(payload, 56)?,
        storage_available_bytes: read_u64(payload, 64)?,
        network_bandwidth_mbps: read_u64(payload, 72)?,
        accelerator_capacity: read_u64(payload, 80)?,
        accelerator_available: read_u64(payload, 88)?,
    })
}

pub(crate) fn encode_audit_events(list: AuditEventList, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let required = AUDIT_HEADER_BYTES + list.events().count() * AUDIT_EVENT_BYTES;
    if output.len() < required { return Err(ProtocolError::BufferTooSmall) }
    output[..required].fill(0);
    write_u64(output, 0, list.generation)?;
    write_u16(output, 16, list.events().count() as u16)?;
    for (index, event) in list.events().enumerate() {
        let offset = AUDIT_HEADER_BYTES + index * AUDIT_EVENT_BYTES;
        write_u64(output, offset, event.sequence)?;
        write_u64(output, offset + 8, event.timestamp_us)?;
        write_u128(output, offset + 16, event.correlation)?;
        write_u32(output, offset + 32, event.actor.raw())?;
        write_u16(output, offset + 36, event.operation)?;
        write_u16(output, offset + 38, event.status)?;
        write_u64(output, offset + 40, event.target)?;
    }
    Ok(required)
}

fn decode_audit_events(frame: &[u8], header: FrameHeader) -> Result<AuditEventList, ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < AUDIT_HEADER_BYTES || header.payload_bytes as usize != payload.len() { return Err(ProtocolError::InvalidFrame) }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_CLUSTER_AUDIT_EVENTS || payload.len() != AUDIT_HEADER_BYTES + count * AUDIT_EVENT_BYTES { return Err(ProtocolError::InvalidFrame) }
    let mut list = AuditEventList::new(read_u64(payload, 0)?);
    for index in 0..count {
        let offset = AUDIT_HEADER_BYTES + index * AUDIT_EVENT_BYTES;
        list.push(crate::ClusterAuditEvent {
            sequence: read_u64(payload, offset)?,
            timestamp_us: read_u64(payload, offset + 8)?,
            correlation: read_u128(payload, offset + 16)?,
            actor: NodeId::new(read_u32(payload, offset + 32)?).ok_or(ProtocolError::InvalidValue)?,
            operation: read_u16(payload, offset + 36)?,
            status: read_u16(payload, offset + 38)?,
            target: read_u64(payload, offset + 40)?,
        })?;
    }
    Ok(list)
}

pub(crate) fn encode_lifecycle_receipt(receipt: LifecycleReceipt, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < LIFECYCLE_RECEIPT_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..LIFECYCLE_RECEIPT_BYTES].fill(0);
    write_u64(output, 0, receipt.operation)?;
    write_u64(output, 8, receipt.generation)?;
    output[16] = receipt.lifecycle as u8;
    Ok(LIFECYCLE_RECEIPT_BYTES)
}

fn decode_lifecycle_receipt(frame: &[u8], header: FrameHeader) -> Result<LifecycleReceipt, ProtocolError> {
    let payload = cluster_payload(frame, header, LIFECYCLE_RECEIPT_BYTES)?;
    Ok(LifecycleReceipt { operation: read_u64(payload, 0)?, generation: read_u64(payload, 8)?, lifecycle: crate::ClusterLifecycle::from_wire(payload[16])? })
}

pub(crate) fn encode_cluster_create(request: ClusterCreateRequest, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < CLUSTER_CREATE_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..CLUSTER_CREATE_BYTES].fill(0);
    write_u128(output, 0, request.cluster.raw())?;
    write(output, 16, &request.name.as_bytes())?;
    Ok(CLUSTER_CREATE_BYTES)
}

pub(crate) fn decode_cluster_create(input: &[u8]) -> Result<ClusterCreateRequest, ProtocolError> {
    if input.len() != CLUSTER_CREATE_BYTES { return Err(ProtocolError::InvalidFrame) }
    Ok(ClusterCreateRequest { cluster: ClusterId::new(read_u128(input, 0)?).ok_or(ProtocolError::InvalidValue)?, name: crate::ClusterName::from_wire(&input[16..80])? })
}

pub(crate) fn encode_cluster_join(request: ClusterJoinRequest, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < CLUSTER_JOIN_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..CLUSTER_JOIN_BYTES].fill(0);
    write_u128(output, 0, request.cluster.raw())?;
    write_u64(output, 16, request.invitation)?;
    write_u32(output, 24, request.node.raw())?;
    Ok(CLUSTER_JOIN_BYTES)
}

pub(crate) fn decode_cluster_join(input: &[u8]) -> Result<ClusterJoinRequest, ProtocolError> {
    if input.len() != CLUSTER_JOIN_BYTES { return Err(ProtocolError::InvalidFrame) }
    Ok(ClusterJoinRequest { cluster: ClusterId::new(read_u128(input, 0)?).ok_or(ProtocolError::InvalidValue)?, invitation: read_u64(input, 16)?, node: NodeId::new(read_u32(input, 24)?).ok_or(ProtocolError::InvalidValue)? })
}

pub(crate) fn encode_cluster_leave(request: ClusterLeaveRequest, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < CLUSTER_LEAVE_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..CLUSTER_LEAVE_BYTES].fill(0);
    write_u128(output, 0, request.cluster.raw())?;
    write_u32(output, 16, request.node.raw())?;
    output[20] = request.force as u8;
    Ok(CLUSTER_LEAVE_BYTES)
}

pub(crate) fn decode_cluster_leave(input: &[u8]) -> Result<ClusterLeaveRequest, ProtocolError> {
    if input.len() != CLUSTER_LEAVE_BYTES { return Err(ProtocolError::InvalidFrame) }
    Ok(ClusterLeaveRequest { cluster: ClusterId::new(read_u128(input, 0)?).ok_or(ProtocolError::InvalidValue)?, node: NodeId::new(read_u32(input, 16)?).ok_or(ProtocolError::InvalidValue)?, force: input[20] != 0 })
}

pub(crate) fn encode_cluster_remove(request: ClusterRemoveRequest, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < CLUSTER_REMOVE_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..CLUSTER_REMOVE_BYTES].fill(0);
    write_u128(output, 0, request.cluster.raw())?;
    write_u64(output, 16, request.confirmation)?;
    Ok(CLUSTER_REMOVE_BYTES)
}

pub(crate) fn decode_cluster_remove(input: &[u8]) -> Result<ClusterRemoveRequest, ProtocolError> {
    if input.len() != CLUSTER_REMOVE_BYTES { return Err(ProtocolError::InvalidFrame) }
    Ok(ClusterRemoveRequest { cluster: ClusterId::new(read_u128(input, 0)?).ok_or(ProtocolError::InvalidValue)?, confirmation: read_u64(input, 16)? })
}

pub(crate) fn encode_subscription(subscription: Subscription, output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.len() < SUBSCRIPTION_BYTES { return Err(ProtocolError::BufferTooSmall) }
    output[..SUBSCRIPTION_BYTES].fill(0);
    output[0] = subscription.kind as u8;
    write_u64(output, 8, subscription.cursor)?;
    Ok(SUBSCRIPTION_BYTES)
}

fn decode_subscription(frame: &[u8], header: FrameHeader) -> Result<Subscription, ProtocolError> {
    let payload = cluster_payload(frame, header, SUBSCRIPTION_BYTES)?;
    Ok(Subscription { kind: crate::SubscriptionKind::from_wire(payload[0])?, cursor: read_u64(payload, 8)? })
}

pub(crate) fn encode_change_batch(batch: ChangeBatch, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let count = batch.events().count();
    let required = CHANGE_HEADER_BYTES + count * CHANGE_EVENT_BYTES;
    if output.len() < required { return Err(ProtocolError::BufferTooSmall) }
    output[..required].fill(0);
    output[0] = batch.subscription.kind as u8;
    output[1] = batch.has_more as u8;
    write_u64(output, 8, batch.next_cursor)?;
    write_u16(output, 16, count as u16)?;
    for (index, event) in batch.events().enumerate() {
        let offset = CHANGE_HEADER_BYTES + index * CHANGE_EVENT_BYTES;
        write_u64(output, offset, event.sequence)?;
        write_u64(output, offset + 8, event.generation)?;
        output[offset + 16] = event.kind as u8;
        write_u16(output, offset + 18, event.operation)?;
        write_u64(output, offset + 24, event.timestamp_us)?;
    }
    Ok(required)
}

fn decode_change_batch(frame: &[u8], header: FrameHeader) -> Result<ChangeBatch, ProtocolError> {
    let payload = frame.get(FRAME_HEADER_BYTES..).ok_or(ProtocolError::InvalidFrame)?;
    if payload.len() < CHANGE_HEADER_BYTES || header.payload_bytes as usize != payload.len() { return Err(ProtocolError::InvalidFrame) }
    let count = read_u16(payload, 16)? as usize;
    if count > crate::MAX_CLUSTER_CHANGES || payload.len() != CHANGE_HEADER_BYTES + count * CHANGE_EVENT_BYTES { return Err(ProtocolError::InvalidFrame) }
    let kind = crate::SubscriptionKind::from_wire(payload[0])?;
    let mut batch = ChangeBatch::empty(Subscription { kind, cursor: read_u64(payload, 8)? });
    batch.next_cursor = read_u64(payload, 8)?;
    batch.has_more = payload[1] != 0;
    for index in 0..count {
        let offset = CHANGE_HEADER_BYTES + index * CHANGE_EVENT_BYTES;
        batch.events[index] = Some(crate::ChangeEvent { sequence: read_u64(payload, offset)?, generation: read_u64(payload, offset + 8)?, kind: crate::SubscriptionKind::from_wire(payload[offset + 16])?, operation: read_u16(payload, offset + 18)?, timestamp_us: read_u64(payload, offset + 24)? });
    }
    Ok(batch)
}
