use ghostos_auth::CryptographicCapability;
use ghostos_system_model::performance::{
    PerformanceBudget, PerformanceDiagnostics, TailLatencyWindow,
};

use crate::{
    AuditEventList, CapabilityDelegation, ChangeBatch, ClusterCreateRequest, ClusterHealthSnapshot,
    ClusterJoinRequest, ClusterLeaveRequest, ClusterRemoveRequest, ClusterResources, ClusterState,
    ClusterSummary, FrameHeader, InvitationList, JobReceipt, JobSpec, JoinPlan, LeavePlan,
    LifecycleReceipt, MemberList, Method, ProtocolError, Subscription, TopologyState,
    RpcStatus,
    client::{
        decode_delegation, decode_job_spec, encode_capability, encode_cluster_state,
        encode_audit_events, encode_change_batch, encode_cluster_health, encode_cluster_resources,
        encode_cluster_summary, encode_invitation_list, encode_job_receipt, encode_join_plan,
        encode_leave_plan, encode_lifecycle_receipt, encode_member_list, encode_subscription,
        encode_topology_state, decode_cluster_create, decode_cluster_join, decode_cluster_leave,
        decode_cluster_remove,
    },
    wire::{
        FLAG_CAPABILITY, FRAME_HEADER_BYTES, PERFORMANCE_DIAGNOSTICS_BYTES,
        encode_performance_diagnostics, read_array,
    },
};

pub const MAX_RPC_METHODS: usize = 18;

pub trait PerformanceClock {
    fn now_us(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopPerformanceClock;

impl PerformanceClock for NoopPerformanceClock {
    fn now_us(&self) -> u64 {
        0
    }
}

pub trait GatewayService {
    fn performance_budget(&self, method: Method) -> PerformanceBudget {
        PerformanceBudget::for_route(method as u16)
    }

    fn performance_queue_wait_us(&self, _method: Method, _request_id: u64) -> u64 {
        0
    }

    fn performance_retries(&self, _method: Method, _request_id: u64) -> u32 {
        0
    }

    fn cluster_state(
        &mut self,
        authority: Option<CryptographicCapability>,
    ) -> Result<ClusterState, RpcStatus>;

    fn topology_state(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<TopologyState, RpcStatus> {
        Ok(TopologyState::new(0, 0))
    }

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

    fn cluster_summary(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<ClusterSummary, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_members(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<MemberList, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_invitations(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<InvitationList, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_join_plan(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<JoinPlan, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_leave_plan(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<LeavePlan, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_health(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<ClusterHealthSnapshot, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_resources(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<ClusterResources, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn cluster_audit(
        &mut self,
        _authority: Option<CryptographicCapability>,
    ) -> Result<AuditEventList, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn create_cluster(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _request: ClusterCreateRequest,
    ) -> Result<LifecycleReceipt, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn join_cluster(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _request: ClusterJoinRequest,
    ) -> Result<LifecycleReceipt, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn leave_cluster(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _request: ClusterLeaveRequest,
    ) -> Result<LifecycleReceipt, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn remove_cluster(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _request: ClusterRemoveRequest,
    ) -> Result<LifecycleReceipt, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn subscribe(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _kind: crate::SubscriptionKind,
    ) -> Result<Subscription, RpcStatus> {
        Err(RpcStatus::NotFound)
    }

    fn poll(
        &mut self,
        _authority: Option<CryptographicCapability>,
        _subscription: Subscription,
        _limit: u8,
    ) -> Result<ChangeBatch, RpcStatus> {
        Err(RpcStatus::NotFound)
    }
}

pub struct FrontendGateway<S, C = NoopPerformanceClock> {
    service: S,
    clock: C,
    tail_windows: [TailLatencyWindow<32>; MAX_RPC_METHODS],
}

impl<S> FrontendGateway<S> {
    pub const fn new(service: S) -> Self {
        Self::with_noop_clock(service)
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

impl<S> FrontendGateway<S, NoopPerformanceClock> {
    pub const fn with_noop_clock(service: S) -> Self {
        Self {
            service,
            clock: NoopPerformanceClock,
            tail_windows: [const { TailLatencyWindow::new() }; MAX_RPC_METHODS],
        }
    }
}

impl<S, C: PerformanceClock> FrontendGateway<S, C> {
    pub const fn with_clock(service: S, clock: C) -> Self {
        Self {
            service,
            clock,
            tail_windows: [const { TailLatencyWindow::new() }; MAX_RPC_METHODS],
        }
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }
}

impl<S: GatewayService, C: PerformanceClock> FrontendGateway<S, C> {
    pub fn handle(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, ProtocolError> {
        if response.len() < FRAME_HEADER_BYTES {
            return Err(ProtocolError::BufferTooSmall);
        }
        let request_header = match FrameHeader::decode(request) {
            Ok(header) => header,
            Err(ProtocolError::AbiMismatch) => {
                return encode_rejection(response, request, RpcStatus::ProtocolMismatch)
            }
            Err(error) => return Err(error),
        };
        if request.len() != FRAME_HEADER_BYTES + request_header.payload_bytes as usize
            || request_header.status != RpcStatus::Ok
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let payload = &request[FRAME_HEADER_BYTES..];
        let (authority, body) = decode_authority(request_header.flags, payload)?;
        let request_started_us = self.clock.now_us();
        let queue_wait_us = self
            .service
            .performance_queue_wait_us(request_header.method, request_header.request_id);
        let service_started_us = self.clock.now_us();
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
            Method::TopologyState => {
                if !body.is_empty() {
                    Err(RpcStatus::InvalidRequest)
                } else {
                    self.service.topology_state(authority).and_then(|state| {
                        encode_topology_state(state, &mut response[FRAME_HEADER_BYTES..])
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
            Method::ClusterSummary => empty_request(body).and_then(|()| self.service.cluster_summary(authority)).and_then(|value| encode_cluster_summary(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterMembers => empty_request(body).and_then(|()| self.service.cluster_members(authority)).and_then(|value| encode_member_list(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterInvitations => empty_request(body).and_then(|()| self.service.cluster_invitations(authority)).and_then(|value| encode_invitation_list(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterJoinPlan => empty_request(body).and_then(|()| self.service.cluster_join_plan(authority)).and_then(|value| encode_join_plan(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterLeavePlan => empty_request(body).and_then(|()| self.service.cluster_leave_plan(authority)).and_then(|value| encode_leave_plan(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterHealth => empty_request(body).and_then(|()| self.service.cluster_health(authority)).and_then(|value| encode_cluster_health(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterResources => empty_request(body).and_then(|()| self.service.cluster_resources(authority)).and_then(|value| encode_cluster_resources(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterAudit => empty_request(body).and_then(|()| self.service.cluster_audit(authority)).and_then(|value| encode_audit_events(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterCreate => decode_cluster_create(body).map_err(|_| RpcStatus::InvalidRequest).and_then(|request| self.service.create_cluster(authority, request)).and_then(|value| encode_lifecycle_receipt(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterJoin => decode_cluster_join(body).map_err(|_| RpcStatus::InvalidRequest).and_then(|request| self.service.join_cluster(authority, request)).and_then(|value| encode_lifecycle_receipt(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterLeave => decode_cluster_leave(body).map_err(|_| RpcStatus::InvalidRequest).and_then(|request| self.service.leave_cluster(authority, request)).and_then(|value| encode_lifecycle_receipt(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::ClusterRemove => decode_cluster_remove(body).map_err(|_| RpcStatus::InvalidRequest).and_then(|request| self.service.remove_cluster(authority, request)).and_then(|value| encode_lifecycle_receipt(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
            Method::Subscribe => {
                decode_subscription_body(body).and_then(|kind| self.service.subscribe(authority, kind)).and_then(|value| encode_subscription(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal))
            }
            Method::Poll => decode_poll(body).and_then(|(subscription, limit)| self.service.poll(authority, subscription, limit)).and_then(|value| encode_change_batch(value, &mut response[FRAME_HEADER_BYTES..]).map_err(|_| RpcStatus::Internal)),
        };
        let (status, payload_bytes) = match outcome {
            Ok(payload_bytes) => (RpcStatus::Ok, payload_bytes),
            Err(status) => (status, 0),
        };
        let service_time_us = self.clock.now_us().saturating_sub(service_started_us);
        let total_latency_us = self.clock.now_us().saturating_sub(request_started_us);
        let method_index = (request_header.method as usize)
            .saturating_sub(1)
            .min(MAX_RPC_METHODS - 1);
        self.tail_windows[method_index].record(total_latency_us);
        let budget = self.service.performance_budget(request_header.method);
        let response_length = FRAME_HEADER_BYTES
            .checked_add(payload_bytes)
            .and_then(|length| length.checked_add(PERFORMANCE_DIAGNOSTICS_BYTES))
            .ok_or(ProtocolError::BufferTooSmall)?;
        if response.len() < response_length {
            return Err(ProtocolError::BufferTooSmall)
        }
        let diagnostics = PerformanceDiagnostics::new(
            queue_wait_us,
            service_time_us,
            self.service
                .performance_retries(request_header.method, request_header.request_id),
            request.len() as u32,
            response_length as u32,
            self.tail_windows[method_index].p99(),
            budget,
        );
        FrameHeader {
            method: request_header.method,
            flags: 0,
            request_id: request_header.request_id,
            payload_bytes: (payload_bytes + PERFORMANCE_DIAGNOSTICS_BYTES) as u32,
            status,
        }
        .encode(response)?;
        encode_performance_diagnostics(
            diagnostics,
            &mut response[FRAME_HEADER_BYTES + payload_bytes..response_length],
        )?;
        Ok(response_length)
    }
}

fn encode_rejection(
    response: &mut [u8],
    request: &[u8],
    status: RpcStatus,
) -> Result<usize, ProtocolError> {
    let method = request
        .get(5)
        .and_then(|raw| Method::from_wire(*raw).ok())
        .unwrap_or(Method::ClusterState);
    let request_id = request
        .get(8..16)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map(u64::from_be_bytes)
        .unwrap_or(0);
    FrameHeader {
        method,
        flags: 0,
        request_id,
        payload_bytes: 0,
        status,
    }
    .encode(response)?;
    Ok(FRAME_HEADER_BYTES)
}

fn empty_request(body: &[u8]) -> Result<(), RpcStatus> {
    if body.is_empty() { Ok(()) } else { Err(RpcStatus::InvalidRequest) }
}

fn decode_subscription_body(body: &[u8]) -> Result<crate::SubscriptionKind, RpcStatus> {
    if body.len() != 8 { return Err(RpcStatus::InvalidRequest) }
    crate::SubscriptionKind::from_wire(body[0]).map_err(|_| RpcStatus::InvalidRequest)
}

fn decode_poll(body: &[u8]) -> Result<(Subscription, u8), RpcStatus> {
    if body.len() != 16 || body[1] == 0 || body[1] as usize > crate::MAX_CLUSTER_CHANGES {
        return Err(RpcStatus::InvalidRequest)
    }
    let kind = crate::SubscriptionKind::from_wire(body[0]).map_err(|_| RpcStatus::InvalidRequest)?;
    let cursor = u64::from_be_bytes(body[8..16].try_into().map_err(|_| RpcStatus::InvalidRequest)?);
    Ok((Subscription { kind, cursor }, body[1]))
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
