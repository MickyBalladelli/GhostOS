use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};

pub const MAX_TIER_OBJECTS: usize = 64;
pub const MAX_TIER_MOVES: usize = 64;
pub const TIERING_STATE_FILE: &str = "SYS$SYSTEM:TIERING.DAT;1";
pub const TIERING_MAGIC: &[u8; 8] = b"SYNTIER1";
pub const TIERING_FORMAT_VERSION: u16 = 1;
pub const TIERING_HEADER_BYTES: usize = 64;
pub const TIER_OBJECT_BYTES: usize = 48;
pub const TIER_MOVE_BYTES: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum StorageTier {
    LocalNvme = 1,
    SlowerDisk = 2,
    RemoteStorage = 3,
    DisposableCache = 4,
}

impl StorageTier {
    pub const fn cost(self) -> u32 {
        match self {
            Self::LocalNvme => 1,
            Self::SlowerDisk => 2,
            Self::RemoteStorage => 4,
            Self::DisposableCache => 0,
        }
    }

    pub const fn maximum_durability(self) -> DurabilityClass {
        match self {
            Self::LocalNvme | Self::SlowerDisk => DurabilityClass::Durable,
            Self::RemoteStorage => DurabilityClass::Replicated,
            Self::DisposableCache => DurabilityClass::Volatile,
        }
    }

    pub const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::LocalNvme,
            2 => Self::SlowerDisk,
            3 => Self::RemoteStorage,
            4 => Self::DisposableCache,
            _ => return None,
        })
    }

    pub const fn supports(self, durability: DurabilityClass) -> bool {
        self.maximum_durability() as u8 >= durability as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum DurabilityClass {
    Volatile = 0,
    Durable = 1,
    Replicated = 2,
}

impl DurabilityClass {
    pub const EPHEMERAL: Self = Self::Volatile;

    pub const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Volatile,
            1 => Self::Durable,
            2 => Self::Replicated,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeatPolicy {
    pub promote_at: u64,
    pub demote_below: u64,
}

impl HeatPolicy {
    pub const fn validate(self) -> Result<Self, TieringError> {
        if self.promote_at == 0 || self.promote_at <= self.demote_below {
            Err(TieringError::InvalidPolicy)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CostPolicy {
    pub maximum: u32,
    pub promote_maximum: u32,
    pub demote_maximum: u32,
}

impl CostPolicy {
    pub const fn validate(self) -> Result<Self, TieringError> {
        if self.promote_maximum > self.maximum || self.demote_maximum > self.maximum {
            Err(TieringError::InvalidPolicy)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurabilityPolicy {
    pub minimum: DurabilityClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TieringPolicy {
    pub heat: HeatPolicy,
    pub cost: CostPolicy,
    pub durability: DurabilityPolicy,
    pub promote_to: StorageTier,
    pub demote_to: StorageTier,
}

impl TieringPolicy {
    pub const DEFAULT: Self = Self {
        heat: HeatPolicy {
            promote_at: 100,
            demote_below: 10,
        },
        cost: CostPolicy {
            maximum: 4,
            promote_maximum: 1,
            demote_maximum: 4,
        },
        durability: DurabilityPolicy {
            minimum: DurabilityClass::Volatile,
        },
        promote_to: StorageTier::LocalNvme,
        demote_to: StorageTier::DisposableCache,
    };

    pub fn validate(self) -> Result<Self, TieringError> {
        self.heat.validate()?;
        self.cost.validate()?;
        if !self.promote_to.supports(self.durability.minimum)
            || !self.demote_to.supports(self.durability.minimum)
            || self.promote_to.cost() > self.cost.promote_maximum
            || self.demote_to.cost() > self.cost.demote_maximum
        {
            Err(TieringError::InvalidPolicy)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierObject {
    pub id: u64,
    pub tier: StorageTier,
    pub declared_durability: DurabilityClass,
    pub heat: u64,
    pub last_access: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TierMoveKind {
    Promotion = 1,
    Demotion = 2,
}

impl TierMoveKind {
    const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Promotion,
            2 => Self::Demotion,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TierMoveState {
    Prepared = 1,
    Committed = 2,
    Cleaned = 3,
    Aborted = 4,
}

impl TierMoveState {
    const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Prepared,
            2 => Self::Committed,
            3 => Self::Cleaned,
            4 => Self::Aborted,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierMoveRecord {
    pub sequence: u64,
    pub object: u64,
    pub source: StorageTier,
    pub destination: StorageTier,
    pub declared_durability: DurabilityClass,
    pub source_generation: u64,
    pub destination_generation: u64,
    pub kind: TierMoveKind,
    pub state: TierMoveState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierMovePlan {
    pub object: u64,
    pub source: StorageTier,
    pub destination: StorageTier,
    pub declared_durability: DurabilityClass,
    pub source_generation: u64,
    pub kind: TierMoveKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierMoveReceipt {
    pub sequence: u64,
    pub object: u64,
    pub source: StorageTier,
    pub destination: StorageTier,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TieringError {
    InvalidPolicy,
    Capacity,
    Duplicate,
    NotFound,
    DurabilityViolation,
    CostLimit,
    StalePlan,
    JournalFull,
    Corrupt,
    BufferTooSmall { required: usize },
    Persistence,
    Backend(u16),
    Interrupted,
}

pub trait TierBackend {
    /// Copy must not destroy the source. The source remains authoritative until
    /// the manager commits the durable destination record.
    fn copy(&mut self, object: u64, source: StorageTier, destination: StorageTier) -> Result<(), u16>;
    /// Complete only after the destination copy is durable at its tier.
    fn flush(&mut self, destination: StorageTier) -> Result<(), u16>;
    /// Removing an old copy is cleanup and must be safe to retry after recovery.
    fn remove(&mut self, object: u64, source: StorageTier) -> Result<(), u16>;
}

pub struct TieringManager<const OBJECTS: usize = MAX_TIER_OBJECTS, const MOVES: usize = MAX_TIER_MOVES> {
    policy: TieringPolicy,
    objects: [Option<TierObject>; OBJECTS],
    moves: [Option<TierMoveRecord>; MOVES],
    next_sequence: u64,
}

impl<const OBJECTS: usize, const MOVES: usize> TieringManager<OBJECTS, MOVES> {
    pub fn new(policy: TieringPolicy) -> Result<Self, TieringError> {
        policy.validate()?;
        Ok(Self {
            policy,
            objects: [None; OBJECTS],
            moves: [None; MOVES],
            next_sequence: 1,
        })
    }

    pub fn with_default_policy() -> Self {
        Self::new(TieringPolicy::DEFAULT).expect("default tiering policy is valid")
    }

    pub const fn policy(&self) -> TieringPolicy {
        self.policy
    }

    pub fn register(
        &mut self,
        id: u64,
        tier: StorageTier,
        declared_durability: DurabilityClass,
        heat: u64,
        last_access: u64,
    ) -> Result<TierObject, TieringError> {
        if tier.cost() > self.policy.cost.maximum
            || !tier.supports(declared_durability)
            || declared_durability < self.policy.durability.minimum
        {
            return Err(TieringError::DurabilityViolation)
        }
        if self.objects.iter().flatten().any(|object| object.id == id) {
            return Err(TieringError::Duplicate)
        }
        let object = TierObject {
            id,
            tier,
            declared_durability,
            heat,
            last_access,
            generation: 1,
        };
        let slot = self
            .objects
            .iter_mut()
            .find(|object| object.is_none())
            .ok_or(TieringError::Capacity)?;
        *slot = Some(object);
        Ok(object)
    }

    pub fn object(&self, id: u64) -> Option<TierObject> {
        self.objects.iter().flatten().find(|object| object.id == id).copied()
    }

    pub fn list(&self, output: &mut [TierObject]) -> usize {
        let mut written = 0;
        for object in self.objects.iter().flatten() {
            let Some(destination) = output.get_mut(written) else { break };
            *destination = *object;
            written += 1;
        }
        written
    }

    pub fn record_access(&mut self, id: u64, heat: u64, at: u64) -> Result<TierObject, TieringError> {
        let object = self
            .objects
            .iter_mut()
            .flatten()
            .find(|object| object.id == id)
            .ok_or(TieringError::NotFound)?;
        object.heat = object.heat.saturating_add(heat);
        object.last_access = at;
        Ok(*object)
    }

    pub fn cool(&mut self, id: u64, amount: u64) -> Result<TierObject, TieringError> {
        let object = self
            .objects
            .iter_mut()
            .flatten()
            .find(|object| object.id == id)
            .ok_or(TieringError::NotFound)?;
        object.heat = object.heat.saturating_sub(amount);
        Ok(*object)
    }

    pub fn plan(&self, id: u64) -> Result<Option<TierMovePlan>, TieringError> {
        let object = self.object(id).ok_or(TieringError::NotFound)?;
        let (destination, kind, limit) = if object.heat >= self.policy.heat.promote_at {
            (self.policy.promote_to, TierMoveKind::Promotion, self.policy.cost.promote_maximum)
        } else if object.heat <= self.policy.heat.demote_below {
            (self.policy.demote_to, TierMoveKind::Demotion, self.policy.cost.demote_maximum)
        } else {
            return Ok(None)
        };
        if destination == object.tier {
            return Ok(None)
        }
        if destination.cost() > limit || destination.cost() > self.policy.cost.maximum {
            return Err(TieringError::CostLimit)
        }
        if !destination.supports(object.declared_durability) {
            return Err(TieringError::DurabilityViolation)
        }
        Ok(Some(TierMovePlan {
            object: object.id,
            source: object.tier,
            destination,
            declared_durability: object.declared_durability,
            source_generation: object.generation,
            kind,
        }))
    }

    pub fn promote_with_interruption<B: TierBackend, I: InterruptionInjector>(
        &mut self,
        id: u64,
        backend: &mut B,
        injector: &mut I,
    ) -> Result<TierMoveReceipt, TieringError> {
        let plan = self.plan(id)?.ok_or(TieringError::NotFound)?;
        if plan.kind != TierMoveKind::Promotion {
            return Err(TieringError::StalePlan)
        }
        self.execute_with_interruption(plan, backend, injector)
    }

    pub fn promote<B: TierBackend>(
        &mut self,
        id: u64,
        backend: &mut B,
    ) -> Result<TierMoveReceipt, TieringError> {
        let mut no_interruption = NoInterruption;
        self.promote_with_interruption(id, backend, &mut no_interruption)
    }

    pub fn demote_with_interruption<B: TierBackend, I: InterruptionInjector>(
        &mut self,
        id: u64,
        backend: &mut B,
        injector: &mut I,
    ) -> Result<TierMoveReceipt, TieringError> {
        let plan = self.plan(id)?.ok_or(TieringError::NotFound)?;
        if plan.kind != TierMoveKind::Demotion {
            return Err(TieringError::StalePlan)
        }
        self.execute_with_interruption(plan, backend, injector)
    }

    pub fn demote<B: TierBackend>(
        &mut self,
        id: u64,
        backend: &mut B,
    ) -> Result<TierMoveReceipt, TieringError> {
        let mut no_interruption = NoInterruption;
        self.demote_with_interruption(id, backend, &mut no_interruption)
    }

    pub fn execute<B: TierBackend>(
        &mut self,
        plan: TierMovePlan,
        backend: &mut B,
    ) -> Result<TierMoveReceipt, TieringError> {
        let mut no_interruption = NoInterruption;
        self.execute_with_interruption(plan, backend, &mut no_interruption)
    }

    pub fn execute_with_interruption<B: TierBackend, I: InterruptionInjector>(
        &mut self,
        plan: TierMovePlan,
        backend: &mut B,
        injector: &mut I,
    ) -> Result<TierMoveReceipt, TieringError> {
        let object = self.object(plan.object).ok_or(TieringError::NotFound)?;
        if object.tier != plan.source
            || object.generation != plan.source_generation
            || object.declared_durability != plan.declared_durability
            || !plan.destination.supports(object.declared_durability)
        {
            return Err(TieringError::StalePlan)
        }
        let (expected_destination, cost_limit) = match plan.kind {
            TierMoveKind::Promotion => (self.policy.promote_to, self.policy.cost.promote_maximum),
            TierMoveKind::Demotion => (self.policy.demote_to, self.policy.cost.demote_maximum),
        };
        if plan.destination != expected_destination
            || plan.destination.cost() > cost_limit
            || plan.destination.cost() > self.policy.cost.maximum
        {
            return Err(TieringError::CostLimit)
        }
        if self.moves.iter().any(|record| {
            record.is_some_and(|record| {
                record.object == plan.object
                    && matches!(record.state, TierMoveState::Prepared | TierMoveState::Committed)
            })
        }) {
            return Err(TieringError::StalePlan)
        }
        let slot = self
            .moves
            .iter_mut()
            .find(|record| record.is_none())
            .ok_or(TieringError::JournalFull)?;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let record = TierMoveRecord {
            sequence,
            object: plan.object,
            source: plan.source,
            destination: plan.destination,
            declared_durability: plan.declared_durability,
            source_generation: plan.source_generation,
            destination_generation: plan.source_generation.saturating_add(1),
            kind: plan.kind,
            state: TierMoveState::Prepared,
        };
        *slot = Some(record);

        backend
            .copy(plan.object, plan.source, plan.destination)
            .map_err(TieringError::Backend)?;
        backend.flush(plan.destination).map_err(TieringError::Backend)?;
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::Flush) {
            return Err(TieringError::Interrupted)
        }

        if let Some(record) = self.moves.iter_mut().flatten().find(|record| record.sequence == sequence) {
            record.state = TierMoveState::Committed;
        }
        let object = self
            .objects
            .iter_mut()
            .flatten()
            .find(|object| object.id == plan.object)
            .ok_or(TieringError::NotFound)?;
        object.tier = plan.destination;
        object.generation = object.generation.saturating_add(1);
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::JournalRecord) {
            return Err(TieringError::Interrupted)
        }

        backend
            .remove(plan.object, plan.source)
            .map_err(TieringError::Backend)?;
        if let Some(record) = self.moves.iter_mut().flatten().find(|record| record.sequence == sequence) {
            record.state = TierMoveState::Cleaned;
        }
        Ok(TierMoveReceipt {
            sequence,
            object: plan.object,
            source: plan.source,
            destination: plan.destination,
            generation: object.generation,
        })
    }

    /// Reconcile a journal after restart. Prepared and aborted moves leave the
    /// source authoritative. Committed and cleaned moves make the destination
    /// authoritative; cleanup can safely be retried by the backend.
    pub fn recover(&mut self) -> Result<(), TieringError> {
        if self.next_sequence == 0 {
            return Err(TieringError::Corrupt)
        }
        let mut last_sequence = 0;
        for record in self.moves.iter().flatten() {
            if record.sequence <= last_sequence
                || record.source == record.destination
                || !record.source.supports(record.declared_durability)
                || !record.destination.supports(record.declared_durability)
            {
                return Err(TieringError::Corrupt)
            }
            last_sequence = record.sequence;
            let Some(object) = self.objects.iter_mut().flatten().find(|object| object.id == record.object) else {
                return Err(TieringError::Corrupt)
            };
            if object.declared_durability != record.declared_durability {
                return Err(TieringError::DurabilityViolation)
            }
            match record.state {
                TierMoveState::Prepared | TierMoveState::Aborted => {
                    if object.generation > record.source_generation {
                        return Err(TieringError::Corrupt)
                    }
                    object.tier = record.source;
                    object.generation = record.source_generation;
                }
                TierMoveState::Committed | TierMoveState::Cleaned => {
                    if object.generation < record.destination_generation {
                        object.generation = record.destination_generation;
                        object.tier = record.destination;
                    } else if object.generation == record.destination_generation
                        && object.tier != record.destination
                    {
                        return Err(TieringError::Corrupt)
                    }
                }
            }
        }
        if self.next_sequence <= last_sequence {
            return Err(TieringError::Corrupt)
        }
        Ok(())
    }

    /// Finish or roll back copies left by a crash. Removing a prepared
    /// destination is rollback; removing a committed source is cleanup.
    pub fn recover_with_backend<B: TierBackend>(&mut self, backend: &mut B) -> Result<(), TieringError> {
        self.recover()?;
        for index in 0..MOVES {
            let Some(record) = self.moves[index] else { continue };
            match record.state {
                TierMoveState::Prepared => {
                    backend
                        .remove(record.object, record.destination)
                        .map_err(TieringError::Backend)?;
                    if let Some(current) = self.moves.iter_mut().flatten().find(|current| current.sequence == record.sequence) {
                        current.state = TierMoveState::Aborted;
                    }
                }
                TierMoveState::Committed => {
                    backend
                        .remove(record.object, record.source)
                        .map_err(TieringError::Backend)?;
                    if let Some(current) = self.moves.iter_mut().flatten().find(|current| current.sequence == record.sequence) {
                        current.state = TierMoveState::Cleaned;
                    }
                }
                TierMoveState::Cleaned | TierMoveState::Aborted => {}
            }
        }
        Ok(())
    }

    pub const fn encoded_len() -> usize {
        TIERING_HEADER_BYTES + OBJECTS * TIER_OBJECT_BYTES + MOVES * TIER_MOVE_BYTES
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, TieringError> {
        let required = Self::encoded_len();
        if output.len() < required {
            return Err(TieringError::BufferTooSmall { required })
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(TIERING_MAGIC);
        output[8..10].copy_from_slice(&TIERING_FORMAT_VERSION.to_le_bytes());
        output[10..12].copy_from_slice(&(self.objects.iter().flatten().count() as u16).to_le_bytes());
        output[12..14].copy_from_slice(&(self.moves.iter().flatten().count() as u16).to_le_bytes());
        output[16..24].copy_from_slice(&self.policy.heat.promote_at.to_le_bytes());
        output[24..32].copy_from_slice(&self.policy.heat.demote_below.to_le_bytes());
        output[32..36].copy_from_slice(&self.policy.cost.maximum.to_le_bytes());
        output[36..40].copy_from_slice(&self.policy.cost.promote_maximum.to_le_bytes());
        output[40..44].copy_from_slice(&self.policy.cost.demote_maximum.to_le_bytes());
        output[44] = self.policy.durability.minimum as u8;
        output[45] = self.policy.promote_to as u8;
        output[46] = self.policy.demote_to as u8;
        output[48..56].copy_from_slice(&self.next_sequence.to_le_bytes());
        for (index, object) in self.objects.iter().enumerate() {
            if let Some(object) = object {
                let start = TIERING_HEADER_BYTES + index * TIER_OBJECT_BYTES;
                output[start] = 1;
                output[start + 1..start + 9].copy_from_slice(&object.id.to_le_bytes());
                output[start + 9] = object.tier as u8;
                output[start + 10] = object.declared_durability as u8;
                output[start + 16..start + 24].copy_from_slice(&object.heat.to_le_bytes());
                output[start + 24..start + 32].copy_from_slice(&object.generation.to_le_bytes());
                output[start + 32..start + 40].copy_from_slice(&object.last_access.to_le_bytes());
            }
        }
        for (index, record) in self.moves.iter().enumerate() {
            if let Some(record) = record {
                let start = TIERING_HEADER_BYTES + OBJECTS * TIER_OBJECT_BYTES + index * TIER_MOVE_BYTES;
                output[start] = 1;
                output[start + 1] = record.state as u8;
                output[start + 2] = record.kind as u8;
                output[start + 3] = record.source as u8;
                output[start + 4] = record.destination as u8;
                output[start + 5] = record.declared_durability as u8;
                output[start + 8..start + 16].copy_from_slice(&record.object.to_le_bytes());
                output[start + 16..start + 24].copy_from_slice(&record.sequence.to_le_bytes());
                output[start + 24..start + 32].copy_from_slice(&record.source_generation.to_le_bytes());
                output[start + 32..start + 40].copy_from_slice(&record.destination_generation.to_le_bytes());
            }
        }
        let checksum = tiering_checksum(&output[..required]);
        output[56..64].copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, TieringError> {
        let required = Self::encoded_len();
        if input.len() < required
            || &input[..8] != TIERING_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != TIERING_FORMAT_VERSION
        {
            return Err(TieringError::Corrupt)
        }
        let stored = u64::from_le_bytes(input[56..64].try_into().map_err(|_| TieringError::Corrupt)?);
        if tiering_checksum(&input[..required]) != stored {
            return Err(TieringError::Corrupt)
        }
        let object_count = u16::from_le_bytes([input[10], input[11]]) as usize;
        let move_count = u16::from_le_bytes([input[12], input[13]]) as usize;
        if object_count > OBJECTS || move_count > MOVES {
            return Err(TieringError::Corrupt)
        }
        let policy = TieringPolicy {
            heat: HeatPolicy {
                promote_at: u64::from_le_bytes(input[16..24].try_into().map_err(|_| TieringError::Corrupt)?),
                demote_below: u64::from_le_bytes(input[24..32].try_into().map_err(|_| TieringError::Corrupt)?),
            },
            cost: CostPolicy {
                maximum: u32::from_le_bytes(input[32..36].try_into().map_err(|_| TieringError::Corrupt)?),
                promote_maximum: u32::from_le_bytes(input[36..40].try_into().map_err(|_| TieringError::Corrupt)?),
                demote_maximum: u32::from_le_bytes(input[40..44].try_into().map_err(|_| TieringError::Corrupt)?),
            },
            durability: DurabilityPolicy {
                minimum: DurabilityClass::from_raw(input[44]).ok_or(TieringError::Corrupt)?,
            },
            promote_to: StorageTier::from_raw(input[45]).ok_or(TieringError::Corrupt)?,
            demote_to: StorageTier::from_raw(input[46]).ok_or(TieringError::Corrupt)?,
        };
        policy.validate().map_err(|_| TieringError::Corrupt)?;
        let mut manager = Self {
            policy,
            objects: [None; OBJECTS],
            moves: [None; MOVES],
            next_sequence: u64::from_le_bytes(input[48..56].try_into().map_err(|_| TieringError::Corrupt)?),
        };
        for index in 0..object_count {
            let start = TIERING_HEADER_BYTES + index * TIER_OBJECT_BYTES;
            if input[start] != 1 {
                return Err(TieringError::Corrupt)
            }
            let object = TierObject {
                id: u64::from_le_bytes(input[start + 1..start + 9].try_into().map_err(|_| TieringError::Corrupt)?),
                tier: StorageTier::from_raw(input[start + 9]).ok_or(TieringError::Corrupt)?,
                declared_durability: DurabilityClass::from_raw(input[start + 10]).ok_or(TieringError::Corrupt)?,
                heat: u64::from_le_bytes(input[start + 16..start + 24].try_into().map_err(|_| TieringError::Corrupt)?),
                generation: u64::from_le_bytes(input[start + 24..start + 32].try_into().map_err(|_| TieringError::Corrupt)?),
                last_access: u64::from_le_bytes(input[start + 32..start + 40].try_into().map_err(|_| TieringError::Corrupt)?),
            };
            if object.tier.cost() > policy.cost.maximum
                || !object.tier.supports(object.declared_durability)
                || object.declared_durability < policy.durability.minimum
                || manager.objects.iter().flatten().any(|current| current.id == object.id)
            {
                return Err(TieringError::Corrupt)
            }
            manager.objects[index] = Some(object);
        }
        for index in 0..move_count {
            let start = TIERING_HEADER_BYTES + OBJECTS * TIER_OBJECT_BYTES + index * TIER_MOVE_BYTES;
            if input[start] != 1 {
                return Err(TieringError::Corrupt)
            }
            manager.moves[index] = Some(TierMoveRecord {
                state: TierMoveState::from_raw(input[start + 1]).ok_or(TieringError::Corrupt)?,
                kind: TierMoveKind::from_raw(input[start + 2]).ok_or(TieringError::Corrupt)?,
                source: StorageTier::from_raw(input[start + 3]).ok_or(TieringError::Corrupt)?,
                destination: StorageTier::from_raw(input[start + 4]).ok_or(TieringError::Corrupt)?,
                declared_durability: DurabilityClass::from_raw(input[start + 5]).ok_or(TieringError::Corrupt)?,
                object: u64::from_le_bytes(input[start + 8..start + 16].try_into().map_err(|_| TieringError::Corrupt)?),
                sequence: u64::from_le_bytes(input[start + 16..start + 24].try_into().map_err(|_| TieringError::Corrupt)?),
                source_generation: u64::from_le_bytes(input[start + 24..start + 32].try_into().map_err(|_| TieringError::Corrupt)?),
                destination_generation: u64::from_le_bytes(input[start + 32..start + 40].try_into().map_err(|_| TieringError::Corrupt)?),
            });
        }
        manager.recover()?;
        Ok(manager)
    }

    pub fn save_to_ghostfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut ghostos_ghostfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<ghostos_ghostfs::TransactionCommit, TieringError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        transaction
            .create_directory("/system", true)
            .map_err(|_| TieringError::Persistence)?;
        transaction
            .write("/system/tiering.dat", &staging[..length])
            .map_err(|_| TieringError::Persistence)?;
        transaction.commit().map_err(|_| TieringError::Persistence)
    }
}

impl<const OBJECTS: usize, const MOVES: usize> TieringManager<OBJECTS, MOVES> {
    pub fn journal(&self) -> impl Iterator<Item = TierMoveRecord> + '_ {
        self.moves.iter().flatten().copied()
    }
}

fn tiering_checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for (index, byte) in bytes.iter().enumerate() {
        if (56..64).contains(&index) {
            continue
        }
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
