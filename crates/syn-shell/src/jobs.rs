use synos_status::Status;

use crate::{Error, parser::Program};

pub const DEFAULT_JOB_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct JobOwner(u64);

impl JobOwner {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct WorkerId(u32);

impl WorkerId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct JobId(u64);

impl JobId {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self((generation as u64) << 32 | slot as u64)
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobPolicy {
    pub priority: u8,
    pub not_before_us: u64,
    pub max_attempts: u8,
    pub dependency: Option<JobId>,
}

impl JobPolicy {
    pub const fn immediate() -> Self {
        Self {
            priority: 128,
            not_before_us: 0,
            max_attempts: 1,
            dependency: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobInfo {
    pub id: JobId,
    pub owner: JobOwner,
    pub state: JobState,
    pub attempts: u8,
    pub max_attempts: u8,
    pub priority: u8,
    pub status: Option<Status>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobLease {
    pub id: JobId,
    pub worker: WorkerId,
    pub program: Program,
    pub deadline_us: u64,
}

#[derive(Clone, Copy)]
struct JobEntry {
    occupied: bool,
    generation: u32,
    owner: JobOwner,
    state: JobState,
    program: Option<Program>,
    priority: u8,
    not_before_us: u64,
    max_attempts: u8,
    attempts: u8,
    dependency: Option<JobId>,
    worker: Option<WorkerId>,
    lease_deadline_us: u64,
    sequence: u64,
    status: Option<Status>,
}

impl JobEntry {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: JobOwner(0),
        state: JobState::Cancelled,
        program: None,
        priority: 0,
        not_before_us: 0,
        max_attempts: 0,
        attempts: 0,
        dependency: None,
        worker: None,
        lease_deadline_us: 0,
        sequence: 0,
        status: None,
    };
}

/// System-wide bounded queue for background commands and pipeline programs.
///
/// Workers claim jobs with expiring leases. Lost workers cause retry, while
/// dependencies and stable submission sequence keep automated pipelines
/// deterministic.
pub struct JobQueue<const CAPACITY: usize = DEFAULT_JOB_CAPACITY> {
    jobs: [JobEntry; CAPACITY],
    sequence: u64,
}

impl<const CAPACITY: usize> JobQueue<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            jobs: [JobEntry::EMPTY; CAPACITY],
            sequence: 0,
        }
    }

    pub fn submit(
        &mut self,
        owner: JobOwner,
        mut program: Program,
        policy: JobPolicy,
    ) -> Result<JobId, Error> {
        if policy.max_attempts == 0 {
            return Err(Error::InvalidValue)
        }
        if let Some(dependency) = policy.dependency {
            self.valid_slot(dependency)?;
        }
        let slot = self
            .jobs
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::QueueFull)?;
        let generation = self.jobs[slot].generation.wrapping_add(1).max(1);
        self.sequence = self.sequence.wrapping_add(1);
        program.background = false;
        self.jobs[slot] = JobEntry {
            occupied: true,
            generation,
            owner,
            state: JobState::Queued,
            program: Some(program),
            priority: policy.priority,
            not_before_us: policy.not_before_us,
            max_attempts: policy.max_attempts,
            attempts: 0,
            dependency: policy.dependency,
            worker: None,
            lease_deadline_us: 0,
            sequence: self.sequence,
            status: None,
        };
        Ok(JobId::from_parts(slot, generation))
    }

    pub fn claim(
        &mut self,
        worker: WorkerId,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<Option<JobLease>, Error> {
        if lease_duration_us == 0 {
            return Err(Error::InvalidValue)
        }
        self.recover_expired(now_us);
        self.propagate_dependencies();
        let slot = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.occupied
                    && entry.state == JobState::Queued
                    && now_us >= entry.not_before_us
                    && self.dependency_completed(entry.dependency)
            })
            .max_by(|(_, left), (_, right)| {
                left.priority
                    .cmp(&right.priority)
                    .then_with(|| right.sequence.cmp(&left.sequence))
            })
            .map(|(slot, _)| slot);
        let Some(slot) = slot else {
            return Ok(None)
        };
        let entry = &mut self.jobs[slot];
        entry.state = JobState::Running;
        entry.worker = Some(worker);
        entry.attempts = entry.attempts.saturating_add(1);
        entry.lease_deadline_us = now_us.saturating_add(lease_duration_us);
        Ok(Some(JobLease {
            id: JobId::from_parts(slot, entry.generation),
            worker,
            program: entry.program.expect("queued job program invariant"),
            deadline_us: entry.lease_deadline_us,
        }))
    }

    pub fn renew(
        &mut self,
        lease: JobLease,
        now_us: u64,
        duration_us: u64,
    ) -> Result<JobLease, Error> {
        let slot = self.valid_slot(lease.id)?;
        let entry = &mut self.jobs[slot];
        if entry.state != JobState::Running || entry.worker != Some(lease.worker) {
            return Err(Error::NotOwner)
        }
        if now_us >= entry.lease_deadline_us || duration_us == 0 {
            return Err(Error::InvalidHandle)
        }
        entry.lease_deadline_us = now_us.saturating_add(duration_us);
        Ok(JobLease {
            deadline_us: entry.lease_deadline_us,
            ..lease
        })
    }

    pub fn finish(
        &mut self,
        lease: JobLease,
        status: Status,
    ) -> Result<JobState, Error> {
        let slot = self.valid_slot(lease.id)?;
        let entry = &mut self.jobs[slot];
        if entry.state != JobState::Running || entry.worker != Some(lease.worker) {
            return Err(Error::NotOwner)
        }
        entry.worker = None;
        entry.status = Some(status);
        entry.state = if status.is_success() {
            JobState::Completed
        } else if entry.attempts < entry.max_attempts {
            JobState::Queued
        } else {
            JobState::Failed
        };
        Ok(entry.state)
    }

    pub fn cancel(&mut self, id: JobId, owner: JobOwner) -> Result<(), Error> {
        let slot = self.valid_slot(id)?;
        let entry = &mut self.jobs[slot];
        if entry.owner != owner {
            return Err(Error::NotOwner)
        }
        if matches!(entry.state, JobState::Completed | JobState::Failed) {
            return Err(Error::InvalidHandle)
        }
        entry.state = JobState::Cancelled;
        entry.worker = None;
        entry.status = Some(Status::BUSY);
        Ok(())
    }

    /// Stop is the command-facing name for owner-guarded job cancellation.
    pub fn stop(&mut self, id: JobId, owner: JobOwner) -> Result<(), Error> {
        self.cancel(id, owner)
    }

    /// Adjust a queued or running job's dispatch priority. The owner token is
    /// required so another session cannot retune or starve the job.
    pub fn set_priority(
        &mut self,
        id: JobId,
        owner: JobOwner,
        priority: u8,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(id)?;
        let entry = &mut self.jobs[slot];
        if entry.owner != owner {
            return Err(Error::NotOwner)
        }
        if priority == 0 {
            return Err(Error::InvalidValue)
        }
        if matches!(entry.state, JobState::Completed | JobState::Failed | JobState::Cancelled) {
            return Err(Error::InvalidHandle)
        }
        entry.priority = priority;
        Ok(())
    }

    pub fn info(&self, id: JobId) -> Result<JobInfo, Error> {
        let entry = &self.jobs[self.valid_slot(id)?];
        Ok(JobInfo {
            id,
            owner: entry.owner,
            state: entry.state,
            attempts: entry.attempts,
            max_attempts: entry.max_attempts,
            priority: entry.priority,
            status: entry.status,
        })
    }

    pub fn reap(&mut self, id: JobId, owner: JobOwner) -> Result<(), Error> {
        let slot = self.valid_slot(id)?;
        let entry = &mut self.jobs[slot];
        if entry.owner != owner {
            return Err(Error::NotOwner)
        }
        if !matches!(
            entry.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err(Error::AlreadyRunning)
        }
        entry.occupied = false;
        entry.program = None;
        Ok(())
    }

    pub fn queued(&self) -> usize {
        self.jobs
            .iter()
            .filter(|entry| entry.occupied && entry.state == JobState::Queued)
            .count()
    }

    pub fn recover_expired(&mut self, now_us: u64) -> usize {
        let mut recovered = 0;
        for entry in &mut self.jobs {
            if entry.occupied
                && entry.state == JobState::Running
                && now_us >= entry.lease_deadline_us
            {
                entry.worker = None;
                entry.status = Some(Status::BUSY);
                entry.state = if entry.attempts < entry.max_attempts {
                    JobState::Queued
                } else {
                    JobState::Failed
                };
                recovered += 1
            }
        }
        recovered
    }

    fn propagate_dependencies(&mut self) {
        for slot in 0..CAPACITY {
            if !self.jobs[slot].occupied || self.jobs[slot].state != JobState::Queued {
                continue
            }
            let Some(dependency) = self.jobs[slot].dependency else {
                continue
            };
            let dependency_state = self
                .valid_slot(dependency)
                .ok()
                .map(|index| self.jobs[index].state);
            if matches!(
                dependency_state,
                Some(JobState::Failed | JobState::Cancelled) | None
            ) {
                self.jobs[slot].state = JobState::Failed;
                self.jobs[slot].status = Some(Status::NOT_FOUND)
            }
        }
    }

    fn dependency_completed(&self, dependency: Option<JobId>) -> bool {
        dependency.is_none_or(|id| {
            self.valid_slot(id)
                .is_ok_and(|slot| self.jobs[slot].state == JobState::Completed)
        })
    }

    fn valid_slot(&self, id: JobId) -> Result<usize, Error> {
        let slot = id.slot();
        let entry = self.jobs.get(slot).ok_or(Error::InvalidHandle)?;
        if !entry.occupied || entry.generation != id.generation() {
            return Err(Error::JobNotFound)
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for JobQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
