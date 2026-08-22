use ghostos_fabric::NodeId;

use crate::{InspectError, Name, PrincipalId};

pub const MAX_USERS: usize = 32;
pub const MAX_SESSIONS: usize = 64;
pub const MAX_PROCESSES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserSample {
    pub principal: PrincipalId,
    pub name: Name<64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionSample {
    pub id: SessionId,
    pub principal: PrincipalId,
    pub node: NodeId,
    pub started_at_us: u64,
    pub last_active_us: u64,
    pub remote: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessSample {
    pub id: ProcessId,
    pub session: SessionId,
    pub principal: PrincipalId,
    pub node: NodeId,
    pub name: Name<64>,
    pub state: ProcessState,
    pub threads: u32,
    pub resident_bytes: u64,
    pub cpu_time_us: u64,
}

#[derive(Clone, Copy)]
pub struct ActivityReport {
    sampled_at_us: u64,
    users: [Option<UserSample>; MAX_USERS],
    sessions: [Option<SessionSample>; MAX_SESSIONS],
    processes: [Option<ProcessSample>; MAX_PROCESSES],
}

impl ActivityReport {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            users: [None; MAX_USERS],
            sessions: [None; MAX_SESSIONS],
            processes: [None; MAX_PROCESSES],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn users(&self) -> impl Iterator<Item = UserSample> + '_ {
        self.users.iter().flatten().copied()
    }

    pub fn sessions(&self) -> impl Iterator<Item = SessionSample> + '_ {
        self.sessions.iter().flatten().copied()
    }

    pub fn processes(&self) -> impl Iterator<Item = ProcessSample> + '_ {
        self.processes.iter().flatten().copied()
    }

    pub fn push_user(&mut self, sample: UserSample) -> Result<(), InspectError> {
        if self.users().any(|entry| entry.principal == sample.principal) {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.users, sample)
    }

    pub fn push_session(&mut self, sample: SessionSample) -> Result<(), InspectError> {
        if sample.last_active_us < sample.started_at_us
            || self.sessions().any(|entry| entry.id == sample.id)
            || !self
                .users()
                .any(|user| user.principal == sample.principal)
        {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.sessions, sample)
    }

    pub fn push_process(&mut self, sample: ProcessSample) -> Result<(), InspectError> {
        let valid_session = self.sessions().any(|session| {
            session.id == sample.session
                && session.principal == sample.principal
                && session.node == sample.node
        });
        if sample.threads == 0
            || self.processes().any(|entry| entry.id == sample.id)
            || !valid_session
        {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.processes, sample)
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }

    pub(crate) fn retain_principal(&mut self, principal: PrincipalId) {
        for entry in &mut self.users {
            if entry.is_some_and(|sample| sample.principal != principal) {
                *entry = None
            }
        }
        for entry in &mut self.sessions {
            if entry.is_some_and(|sample| sample.principal != principal) {
                *entry = None
            }
        }
        for entry in &mut self.processes {
            if entry.is_some_and(|sample| sample.principal != principal) {
                *entry = None
            }
        }
    }
}

impl Default for ActivityReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Fixed-capacity session/process tracker owned by the authentication service.
pub struct ActivityRegistry {
    report: ActivityReport,
}

impl ActivityRegistry {
    pub const fn new() -> Self {
        Self {
            report: ActivityReport::new(),
        }
    }

    pub fn add_user(&mut self, user: UserSample) -> Result<(), InspectError> {
        self.report.push_user(user)
    }

    pub fn open_session(&mut self, session: SessionSample) -> Result<(), InspectError> {
        if !self
            .report
            .users()
            .any(|user| user.principal == session.principal)
        {
            return Err(InspectError::InvalidSample)
        }
        self.report.push_session(session)
    }

    pub fn touch_session(
        &mut self,
        session: SessionId,
        now_us: u64,
    ) -> Result<(), InspectError> {
        let sample = self
            .report
            .sessions
            .iter_mut()
            .flatten()
            .find(|sample| sample.id == session)
            .ok_or(InspectError::NotFound)?;
        if now_us < sample.last_active_us {
            return Err(InspectError::InvalidSample)
        }
        sample.last_active_us = now_us;
        Ok(())
    }

    pub fn close_session(&mut self, session: SessionId) -> Result<(), InspectError> {
        let slot = self
            .report
            .sessions
            .iter_mut()
            .find(|entry| entry.is_some_and(|sample| sample.id == session))
            .ok_or(InspectError::NotFound)?;
        *slot = None;
        for process in &mut self.report.processes {
            if process.is_some_and(|sample| sample.session == session) {
                *process = None
            }
        }
        Ok(())
    }

    pub fn spawn_process(&mut self, process: ProcessSample) -> Result<(), InspectError> {
        let session = self
            .report
            .sessions()
            .find(|session| session.id == process.session)
            .ok_or(InspectError::InvalidSample)?;
        if session.principal != process.principal || session.node != process.node {
            return Err(InspectError::InvalidSample)
        }
        self.report.push_process(process)
    }

    pub fn exit_process(&mut self, process: ProcessId) -> Result<(), InspectError> {
        let slot = self
            .report
            .processes
            .iter_mut()
            .find(|entry| entry.is_some_and(|sample| sample.id == process))
            .ok_or(InspectError::NotFound)?;
        *slot = None;
        Ok(())
    }

    pub fn snapshot(&self, sampled_at_us: u64) -> ActivityReport {
        let mut report = self.report;
        report.set_sampled_at_us(sampled_at_us);
        report
    }
}

impl Default for ActivityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn insert<T: Copy, const CAPACITY: usize>(
    entries: &mut [Option<T>; CAPACITY],
    sample: T,
) -> Result<(), InspectError> {
    let slot = entries
        .iter_mut()
        .find(|entry| entry.is_none())
        .ok_or(InspectError::Capacity)?;
    *slot = Some(sample);
    Ok(())
}
