#![no_std]
#![deny(unsafe_code)]

//! Bounded control-plane state for horizontally scaled Ring 3 services.
//!
//! The controller keeps routing state separate from service payloads. A
//! request has one stable id and one idempotency key, so a retry after a
//! drain, restart, or handoff returns the original receipt instead of running
//! a side effect twice. Sessions move only after in-flight work reaches zero.

#[allow(unsafe_code)]
mod native;

pub const MAX_SCALE_INSTANCES: usize = 32;
pub const MAX_SCALE_SESSIONS: usize = 128;
pub const MAX_SCALE_REQUESTS: usize = 256;
pub const MAX_SCALE_EFFECTS: usize = 256;
pub const MAX_SCALE_SNAPSHOT_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceKind {
    Http = 1,
    RemoteTerminal = 2,
    Package = 3,
    Compiler = 4,
    Storage = 5,
    Observability = 6,
}

impl ServiceKind {
    pub const ALL: [Self; 6] = [
        Self::Http,
        Self::RemoteTerminal,
        Self::Package,
        Self::Compiler,
        Self::Storage,
        Self::Observability,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::RemoteTerminal => "remote-terminal",
            Self::Package => "package",
            Self::Compiler => "compiler",
            Self::Storage => "storage",
            Self::Observability => "observability",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InstanceId(u32);

impl InstanceId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SessionId(u64);

impl SessionId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RequestId(u64);

impl RequestId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceState {
    Joining,
    Ready,
    Draining,
    Restarting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Active,
    HandoffPrepared,
    HandoffAccepted,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestState {
    InFlight,
    Failed,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleError {
    InvalidConfiguration,
    Capacity,
    InvalidId,
    Duplicate,
    NotFound,
    InvalidState,
    StaleGeneration,
    NoTarget,
    InFlight,
    Conflict,
    BufferTooSmall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinReceipt {
    pub instance: InstanceId,
    pub generation: u64,
    pub state: InstanceState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainReport {
    pub instance: InstanceId,
    pub generation: u64,
    pub sessions: usize,
    pub in_flight: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectReceipt {
    pub request: RequestId,
    pub effect: u64,
    pub attempt: u32,
    pub result: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteDecision {
    New {
        instance: InstanceId,
        generation: u64,
        attempt: u32,
    },
    InFlight {
        instance: InstanceId,
        attempt: u32,
    },
    Completed(EffectReceipt),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffToken {
    pub session: SessionId,
    pub source: InstanceId,
    pub target: InstanceId,
    pub source_generation: u64,
    pub target_generation: u64,
    pub sequence: u64,
    pub snapshot_digest: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffReceipt {
    pub session: SessionId,
    pub source: InstanceId,
    pub target: InstanceId,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScaleSnapshot {
    pub service: ServiceKind,
    pub instances: usize,
    pub ready: usize,
    pub draining: usize,
    pub restarting: usize,
    pub sessions: usize,
    pub in_flight: usize,
    pub completed_effects: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InstanceRecord {
    id: InstanceId,
    generation: u64,
    state: InstanceState,
    sessions: u16,
    in_flight: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HandoffRecord<const SNAPSHOT: usize> {
    source: InstanceId,
    target: InstanceId,
    source_generation: u64,
    target_generation: u64,
    sequence: u64,
    digest: u64,
    accepted: bool,
    bytes: [u8; SNAPSHOT],
    length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SessionRecord<const SNAPSHOT: usize> {
    id: SessionId,
    owner: InstanceId,
    generation: u64,
    sequence: u64,
    state: SessionState,
    in_flight: u16,
    handoff: Option<HandoffRecord<SNAPSHOT>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestRecord {
    id: RequestId,
    session: SessionId,
    owner: InstanceId,
    generation: u64,
    effect: u64,
    attempt: u32,
    state: RequestState,
    receipt: Option<EffectReceipt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EffectRecord {
    effect: u64,
    receipt: EffectReceipt,
}

/// Fixed-capacity membership, handoff, routing, and idempotency ledger.
pub struct ServiceScale<
    const INSTANCES: usize = 8,
    const SESSIONS: usize = 64,
    const REQUESTS: usize = 128,
    const EFFECTS: usize = 128,
    const SNAPSHOT: usize = MAX_SCALE_SNAPSHOT_BYTES,
> {
    service: ServiceKind,
    instances: [Option<InstanceRecord>; INSTANCES],
    sessions: [Option<SessionRecord<SNAPSHOT>>; SESSIONS],
    requests: [Option<RequestRecord>; REQUESTS],
    effects: [Option<EffectRecord>; EFFECTS],
}

impl<
        const INSTANCES: usize,
        const SESSIONS: usize,
        const REQUESTS: usize,
        const EFFECTS: usize,
        const SNAPSHOT: usize,
    > ServiceScale<INSTANCES, SESSIONS, REQUESTS, EFFECTS, SNAPSHOT>
{
    pub fn new(service: ServiceKind) -> Result<Self, ScaleError> {
        if INSTANCES == 0 || SESSIONS == 0 || REQUESTS == 0 || EFFECTS == 0 || SNAPSHOT == 0 {
            return Err(ScaleError::InvalidConfiguration)
        }
        Ok(Self {
            service,
            instances: [None; INSTANCES],
            sessions: [None; SESSIONS],
            requests: [None; REQUESTS],
            effects: [None; EFFECTS],
        })
    }

    pub const fn service(&self) -> ServiceKind {
        self.service
    }

    pub fn join(&mut self, instance: InstanceId, generation: u64) -> Result<JoinReceipt, ScaleError> {
        let slot = native::membership(&self.instances, instance, generation, 0)?;
        self.instances[slot] = Some(InstanceRecord {
            id: instance,
            generation,
            state: InstanceState::Joining,
            sessions: 0,
            in_flight: 0,
        });
        Ok(JoinReceipt {
            instance,
            generation,
            state: InstanceState::Joining,
        })
    }

    pub fn ready(&mut self, instance: InstanceId, generation: u64) -> Result<(), ScaleError> {
        let index = native::membership(&self.instances, instance, generation, 1)?;
        let record = self.instances[index].ok_or(ScaleError::NotFound)?;
        self.instances[index] = Some(InstanceRecord {
            state: InstanceState::Ready,
            ..record
        });
        Ok(())
    }

    pub fn drain(
        &mut self,
        instance: InstanceId,
        generation: u64,
    ) -> Result<DrainReport, ScaleError> {
        let index = native::membership(&self.instances, instance, generation, 2)?;
        let record = self.instances[index].ok_or(ScaleError::NotFound)?;
        self.instances[index] = Some(InstanceRecord {
            state: InstanceState::Draining,
            ..record
        });
        Ok(DrainReport {
            instance,
            generation,
            sessions: record.sessions as usize,
            in_flight: record.in_flight as usize,
        })
    }

    pub fn restart(&mut self, instance: InstanceId, generation: u64) -> Result<(), ScaleError> {
        let index = native::membership(&self.instances, instance, generation, 3)?;
        let record = self.instances[index].ok_or(ScaleError::NotFound)?;
        self.instances[index] = Some(InstanceRecord {
            state: InstanceState::Restarting,
            ..record
        });
        Ok(())
    }

    pub fn open_session(
        &mut self,
        session: SessionId,
        instance: InstanceId,
        generation: u64,
    ) -> Result<(), ScaleError> {
        if self.session_index(session).is_some() {
            return Err(ScaleError::Duplicate)
        }
        let instance_index = self.checked_ready_instance(instance, generation)?;
        let slot = self
            .sessions
            .iter()
            .position(Option::is_none)
            .ok_or(ScaleError::Capacity)?;
        self.sessions[slot] = Some(SessionRecord {
            id: session,
            owner: instance,
            generation,
            sequence: 0,
            state: SessionState::Active,
            in_flight: 0,
            handoff: None,
        });
        let record = self.instances[instance_index].ok_or(ScaleError::NotFound)?;
        self.instances[instance_index] = Some(InstanceRecord {
            sessions: record.sessions.checked_add(1).ok_or(ScaleError::Capacity)?,
            ..record
        });
        Ok(())
    }

    pub fn close_session(&mut self, session: SessionId) -> Result<(), ScaleError> {
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        native::close(record.state, record.in_flight)?;
        let instance_index = self
            .instance_index(record.owner)
            .ok_or(ScaleError::NotFound)?;
        let instance = self.instances[instance_index].ok_or(ScaleError::NotFound)?;
        self.instances[instance_index] = Some(InstanceRecord {
            sessions: instance.sessions.saturating_sub(1),
            ..instance
        });
        self.sessions[session_index] = Some(SessionRecord {
            state: SessionState::Closed,
            ..record
        });
        Ok(())
    }

    /// Route or deduplicate one logical operation.
    pub fn route_request(
        &mut self,
        request: RequestId,
        session: SessionId,
        effect: u64,
    ) -> Result<RouteDecision, ScaleError> {
        let (decision, index) = native::route(&self.requests, &self.effects, request, session, effect)?;
        match decision {
            0 => {},
            1 => return self.retry_request(index),
            2 => {
                let record = self.requests[index].ok_or(ScaleError::NotFound)?;
                return Ok(RouteDecision::InFlight { instance: record.owner, attempt: record.attempt })
            }
            3 => return Ok(RouteDecision::Completed(self.requests[index]
                .ok_or(ScaleError::NotFound)?.receipt.ok_or(ScaleError::InvalidState)?)),
            4 => return Ok(RouteDecision::Completed(self.effects[index]
                .ok_or(ScaleError::NotFound)?.receipt)),
            _ => unreachable!("native route decision"),
        }
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let session_record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        if !matches!(session_record.state, SessionState::Active) {
            return Err(ScaleError::InvalidState)
        }
        let instance_index = self.checked_ready_instance(
            session_record.owner,
            session_record.generation,
        )?;
        let request_index = self
            .requests
            .iter()
            .position(Option::is_none)
            .ok_or(ScaleError::Capacity)?;
        let record = RequestRecord {
            id: request,
            session,
            owner: session_record.owner,
            generation: session_record.generation,
            effect,
            attempt: 1,
            state: RequestState::InFlight,
            receipt: None,
        };
        self.requests[request_index] = Some(record);
        self.add_in_flight(instance_index, session_index)?;
        Ok(RouteDecision::New {
            instance: record.owner,
            generation: record.generation,
            attempt: record.attempt,
        })
    }

    pub fn complete_request(
        &mut self,
        request: RequestId,
        instance: InstanceId,
        generation: u64,
        result: u64,
    ) -> Result<EffectReceipt, ScaleError> {
        let request_index = self.request_index(request).ok_or(ScaleError::NotFound)?;
        let record = self.requests[request_index].ok_or(ScaleError::NotFound)?;
        if record.owner != instance || record.generation != generation {
            return Err(ScaleError::StaleGeneration)
        }
        if !matches!(record.state, RequestState::InFlight) {
            return Err(ScaleError::InvalidState)
        }
        if self.effect_receipt(record.effect).is_some() {
            return Err(ScaleError::Conflict)
        }
        let effect_index = self
            .effects
            .iter()
            .position(Option::is_none)
            .ok_or(ScaleError::Capacity)?;
        let receipt = EffectReceipt {
            request,
            effect: record.effect,
            attempt: record.attempt,
            result,
        };
        self.requests[request_index] = Some(RequestRecord {
            state: RequestState::Completed,
            receipt: Some(receipt),
            ..record
        });
        self.finish_in_flight(record.owner, record.session)?;
        self.effects[effect_index] = Some(EffectRecord {
            effect: record.effect,
            receipt,
        });
        Ok(receipt)
    }

    pub fn fail_request(
        &mut self,
        request: RequestId,
        instance: InstanceId,
        generation: u64,
    ) -> Result<(), ScaleError> {
        let request_index = self.request_index(request).ok_or(ScaleError::NotFound)?;
        let record = self.requests[request_index].ok_or(ScaleError::NotFound)?;
        if record.owner != instance || record.generation != generation {
            return Err(ScaleError::StaleGeneration)
        }
        if !matches!(record.state, RequestState::InFlight) {
            return Err(ScaleError::InvalidState)
        }
        self.requests[request_index] = Some(RequestRecord {
            state: RequestState::Failed,
            ..record
        });
        self.finish_in_flight(record.owner, record.session)
    }

    /// Prepare a session snapshot. No new request can route while prepared.
    pub fn prepare_handoff(
        &mut self,
        session: SessionId,
        source_generation: u64,
        target: InstanceId,
        sequence: u64,
        snapshot: &[u8],
    ) -> Result<HandoffToken, ScaleError> {
        if snapshot.is_empty() || snapshot.len() > SNAPSHOT {
            return Err(ScaleError::BufferTooSmall)
        }
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        native::prepare(record.state, record.in_flight, record.generation,
            source_generation, record.sequence, sequence)?;
        self.checked_instance(record.owner, source_generation)?;
        let target_index = self
            .instance_index(target)
            .ok_or(ScaleError::NoTarget)?;
        let target_record = self.instances[target_index].ok_or(ScaleError::NotFound)?;
        if !matches!(target_record.state, InstanceState::Ready) || target == record.owner {
            return Err(ScaleError::NoTarget)
        }
        let mut bytes = [0; SNAPSHOT];
        bytes[..snapshot.len()].copy_from_slice(snapshot);
        let digest = digest(&bytes[..snapshot.len()]);
        let handoff = HandoffRecord {
            source: record.owner,
            target,
            source_generation,
            target_generation: target_record.generation,
            sequence,
            digest,
            accepted: false,
            bytes,
            length: snapshot.len(),
        };
        let token = HandoffToken {
            session,
            source: record.owner,
            target,
            source_generation,
            target_generation: target_record.generation,
            sequence,
            snapshot_digest: digest,
        };
        self.sessions[session_index] = Some(SessionRecord {
            state: SessionState::HandoffPrepared,
            handoff: Some(handoff),
            ..record
        });
        Ok(token)
    }

    pub fn accept_handoff(
        &mut self,
        token: HandoffToken,
        target: InstanceId,
        generation: u64,
    ) -> Result<(), ScaleError> {
        let session_index = self.session_index(token.session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let handoff = record.handoff.ok_or(ScaleError::InvalidState)?;
        if target != token.target
            || generation != token.target_generation
            || !same_handoff(handoff, token)
            || !matches!(record.state, SessionState::HandoffPrepared)
        {
            return Err(ScaleError::StaleGeneration)
        }
        self.checked_ready_instance(target, generation)?;
        self.sessions[session_index] = Some(SessionRecord {
            state: SessionState::HandoffAccepted,
            handoff: Some(HandoffRecord {
                accepted: true,
                ..handoff
            }),
            ..record
        });
        Ok(())
    }

    /// Copy the prepared session state into the target's bounded workspace.
    /// The target must verify and install these bytes before committing.
    pub fn handoff_snapshot(
        &self,
        token: HandoffToken,
        destination: &mut [u8],
    ) -> Result<usize, ScaleError> {
        let session_index = self.session_index(token.session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let handoff = record.handoff.ok_or(ScaleError::InvalidState)?;
        if !matches!(record.state, SessionState::HandoffPrepared | SessionState::HandoffAccepted)
            || !same_handoff(handoff, token)
        {
            return Err(ScaleError::InvalidState)
        }
        if destination.len() < handoff.length {
            return Err(ScaleError::BufferTooSmall)
        }
        destination[..handoff.length].copy_from_slice(&handoff.bytes[..handoff.length]);
        Ok(handoff.length)
    }

    pub fn commit_handoff(&mut self, token: HandoffToken) -> Result<HandoffReceipt, ScaleError> {
        let session_index = self.session_index(token.session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let handoff = record.handoff.ok_or(ScaleError::InvalidState)?;
        if !handoff.accepted
            || !matches!(record.state, SessionState::HandoffAccepted)
            || !same_handoff(handoff, token)
        {
            return Err(ScaleError::InvalidState)
        }
        let source_index = self.checked_instance(handoff.source, handoff.source_generation)?;
        let target_index = self.checked_ready_instance(handoff.target, handoff.target_generation)?;
        let source = self.instances[source_index].ok_or(ScaleError::NotFound)?;
        let target = self.instances[target_index].ok_or(ScaleError::NotFound)?;
        self.instances[source_index] = Some(InstanceRecord {
            sessions: source.sessions.saturating_sub(1),
            ..source
        });
        self.instances[target_index] = Some(InstanceRecord {
            sessions: target.sessions.checked_add(1).ok_or(ScaleError::Capacity)?,
            ..target
        });
        self.sessions[session_index] = Some(SessionRecord {
            owner: handoff.target,
            generation: handoff.target_generation,
            sequence: handoff.sequence,
            state: SessionState::Active,
            handoff: None,
            ..record
        });
        Ok(HandoffReceipt {
            session: token.session,
            source: handoff.source,
            target: handoff.target,
            sequence: handoff.sequence,
        })
    }

    pub fn abort_handoff(&mut self, token: HandoffToken) -> Result<(), ScaleError> {
        let session_index = self.session_index(token.session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let handoff = record.handoff.ok_or(ScaleError::InvalidState)?;
        if !same_handoff(handoff, token) {
            return Err(ScaleError::Conflict)
        }
        self.sessions[session_index] = Some(SessionRecord {
            state: SessionState::Active,
            handoff: None,
            ..record
        });
        Ok(())
    }

    /// Pick the ready instance with the fewest sessions and in-flight calls.
    pub fn target_for(&self, session: SessionId) -> Result<InstanceId, ScaleError> {
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let owner = self.sessions[session_index].ok_or(ScaleError::NotFound)?.owner;
        native::target(&self.instances, owner)
    }

    pub fn rebalance_session(
        &mut self,
        session: SessionId,
        sequence: u64,
        snapshot: &[u8],
    ) -> Result<HandoffReceipt, ScaleError> {
        let target = self.target_for(session)?;
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let token = self.prepare_handoff(session, record.generation, target, sequence, snapshot)?;
        self.accept_handoff(token, target, token.target_generation)?;
        self.commit_handoff(token)
    }

    pub fn session_at(&self, index: usize) -> Option<(SessionId, InstanceId, SessionState)> {
        self.sessions.get(index).and_then(|record| {
            record.map(|record| (record.id, record.owner, record.state))
        })
    }

    pub fn snapshot(&self) -> ScaleSnapshot {
        let mut snapshot = ScaleSnapshot {
            service: self.service,
            instances: 0,
            ready: 0,
            draining: 0,
            restarting: 0,
            sessions: 0,
            in_flight: 0,
            completed_effects: 0,
        };
        for record in self.instances.iter().flatten() {
            snapshot.instances += 1;
            snapshot.sessions += record.sessions as usize;
            snapshot.in_flight += record.in_flight as usize;
            match record.state {
                InstanceState::Ready => snapshot.ready += 1,
                InstanceState::Draining => snapshot.draining += 1,
                InstanceState::Restarting => snapshot.restarting += 1,
                InstanceState::Joining => {}
            }
        }
        snapshot.completed_effects = self.effects.iter().flatten().count();
        snapshot
    }

    fn retry_request(&mut self, index: usize) -> Result<RouteDecision, ScaleError> {
        let record = self.requests[index].ok_or(ScaleError::NotFound)?;
        let session_index = self.session_index(record.session).ok_or(ScaleError::NotFound)?;
        let session = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        let instance_index = self.checked_ready_instance(session.owner, session.generation)?;
        let attempt = record.attempt.checked_add(1).ok_or(ScaleError::Capacity)?;
        let updated = RequestRecord {
            owner: session.owner,
            generation: session.generation,
            attempt,
            state: RequestState::InFlight,
            ..record
        };
        self.requests[index] = Some(updated);
        self.add_in_flight(instance_index, session_index)?;
        Ok(RouteDecision::New {
            instance: updated.owner,
            generation: updated.generation,
            attempt,
        })
    }

    fn add_in_flight(&mut self, instance_index: usize, session_index: usize) -> Result<(), ScaleError> {
        let instance = self.instances[instance_index].ok_or(ScaleError::NotFound)?;
        let session = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        self.instances[instance_index] = Some(InstanceRecord {
            in_flight: instance.in_flight.checked_add(1).ok_or(ScaleError::Capacity)?,
            ..instance
        });
        self.sessions[session_index] = Some(SessionRecord {
            in_flight: session.in_flight.checked_add(1).ok_or(ScaleError::Capacity)?,
            ..session
        });
        Ok(())
    }

    fn finish_in_flight(&mut self, instance: InstanceId, session: SessionId) -> Result<(), ScaleError> {
        let instance_index = self.instance_index(instance).ok_or(ScaleError::NotFound)?;
        let session_index = self.session_index(session).ok_or(ScaleError::NotFound)?;
        let instance_record = self.instances[instance_index].ok_or(ScaleError::NotFound)?;
        let session_record = self.sessions[session_index].ok_or(ScaleError::NotFound)?;
        self.instances[instance_index] = Some(InstanceRecord {
            in_flight: instance_record.in_flight.saturating_sub(1),
            ..instance_record
        });
        self.sessions[session_index] = Some(SessionRecord {
            in_flight: session_record.in_flight.saturating_sub(1),
            ..session_record
        });
        Ok(())
    }

    fn effect_receipt(&self, effect: u64) -> Option<EffectReceipt> {
        self.effects
            .iter()
            .flatten()
            .find(|record| record.effect == effect)
            .map(|record| record.receipt)
    }

    fn instance_index(&self, instance: InstanceId) -> Option<usize> {
        self.instances.iter().position(|record| record.is_some_and(|record| record.id == instance))
    }

    fn session_index(&self, session: SessionId) -> Option<usize> {
        self.sessions.iter().position(|record| record.is_some_and(|record| record.id == session))
    }

    fn request_index(&self, request: RequestId) -> Option<usize> {
        self.requests.iter().position(|record| record.is_some_and(|record| record.id == request))
    }

    fn checked_instance(&self, instance: InstanceId, generation: u64) -> Result<usize, ScaleError> {
        native::membership(&self.instances, instance, generation, 4)
    }

    fn checked_ready_instance(&self, instance: InstanceId, generation: u64) -> Result<usize, ScaleError> {
        native::membership(&self.instances, instance, generation, 5)
    }
}

fn same_handoff<const SNAPSHOT: usize>(handoff: HandoffRecord<SNAPSHOT>, token: HandoffToken) -> bool {
    native::same_handoff(&handoff, token)
        && digest(&handoff.bytes[..handoff.length]) == token.snapshot_digest
}

fn digest(bytes: &[u8]) -> u64 {
    native::digest(bytes)
}

pub type HttpScale = ServiceScale<8, 256, 512, 512, 1024>;
pub type RemoteTerminalScale = ServiceScale<8, 128, 256, 256, 1024>;
pub type PackageScale = ServiceScale<8, 128, 256, 256, 1024>;
pub type CompilerScale = ServiceScale<8, 128, 256, 256, 1024>;
pub type StorageScale = ServiceScale<16, 256, 512, 512, 1024>;
pub type ObservabilityScale = ServiceScale<16, 256, 512, 512, 1024>;

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> InstanceId {
        InstanceId::new(raw).expect("instance")
    }

    fn session(raw: u64) -> SessionId {
        SessionId::new(raw).expect("session")
    }

    fn request(raw: u64) -> RequestId {
        RequestId::new(raw).expect("request")
    }

    #[test]
    fn drain_handoff_restart_preserves_session_and_generation_fences() {
        let mut scale = HttpScale::new(ServiceKind::Http).expect("controller");
        scale.join(id(1), 1).expect("join");
        scale.join(id(2), 1).expect("join");
        scale.ready(id(1), 1).expect("ready");
        scale.ready(id(2), 1).expect("ready");
        scale.open_session(session(7), id(1), 1).expect("session");
        scale.drain(id(1), 1).expect("drain");
        let token = scale
            .prepare_handoff(session(7), 1, id(2), 1, b"terminal-state")
            .expect("prepare");
        let mut snapshot = [0; 32];
        let length = scale
            .handoff_snapshot(token, &mut snapshot)
            .expect("snapshot");
        assert_eq!(&snapshot[..length], b"terminal-state");
        scale.accept_handoff(token, id(2), 1).expect("accept");
        let receipt = scale.commit_handoff(token).expect("commit");
        assert_eq!(receipt.target, id(2));
        scale.restart(id(1), 1).expect("restart");
        scale.join(id(1), 2).expect("rejoin");
        scale.ready(id(1), 2).expect("ready again");
        assert_eq!(scale.snapshot().sessions, 1);
        assert_eq!(scale.snapshot().ready, 2);
        assert_eq!(scale.ready(id(2), 0), Err(ScaleError::StaleGeneration));
    }

    #[test]
    fn retry_and_duplicate_effects_do_not_repeat_side_effect() {
        let mut scale = PackageScale::new(ServiceKind::Package).expect("controller");
        scale.join(id(1), 1).expect("join");
        scale.ready(id(1), 1).expect("ready");
        scale.open_session(session(1), id(1), 1).expect("session");
        assert!(matches!(
            scale.route_request(request(1), session(1), 44),
            Ok(RouteDecision::New { attempt: 1, .. })
        ));
        assert_eq!(
            scale.route_request(request(2), session(1), 44),
            Ok(RouteDecision::InFlight {
                instance: id(1),
                attempt: 1,
            })
        );
        scale.fail_request(request(1), id(1), 1).expect("fail");
        assert!(matches!(
            scale.route_request(request(1), session(1), 44),
            Ok(RouteDecision::New { attempt: 2, .. })
        ));
        let receipt = scale.complete_request(request(1), id(1), 1, 99).expect("complete");
        assert_eq!(
            scale.route_request(request(9), session(1), 44),
            Ok(RouteDecision::Completed(receipt))
        );
        assert_eq!(scale.snapshot().in_flight, 0);
    }

    #[test]
    fn every_target_service_has_a_controller() {
        for service in ServiceKind::ALL {
            let controller = ServiceScale::<1, 1, 1, 1, 1>::new(service).expect("controller");
            assert_eq!(controller.service(), service);
        }
    }
}
