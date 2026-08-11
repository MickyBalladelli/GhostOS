//! Online integrity inspection and explicitly authorized repair planning.
//!
//! Scrubbing takes a shared reference, so it does not stop normal filesystem
//! readers or writers. A plan is only a report until a caller presents an
//! explicit operator authorization. The only automatic repair is reclaiming
//! unreachable CoW blocks; structural and checksum corruption is reported but
//! never guessed at or rewritten.

use super::{Block, BlockId, Error, SynFs, TreeBlock, checksum};

const EMPTY_BLOCK_FINGERPRINT: u64 = 0;

/// Logical owner of the blocks being inspected. These owners share one CoW
/// volume, so the scrub validates the complete graph for every scope while
/// retaining the operator's requested scope in the evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ScrubScope {
    SynFs = 1,
    PackageStore = 2,
    Journals = 3,
    Snapshots = 4,
    SystemMetadata = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrubIssue {
    CorruptBlock,
    DuplicateBlock,
    StaleGeneration,
    TornRecord,
    OrphanedCapability,
    UnreachableBlock,
    InvalidCheckpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrubFinding {
    pub scope: ScrubScope,
    pub block: u32,
    pub issue: ScrubIssue,
    pub before_fingerprint: u64,
    pub repairable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrubReport {
    pub scope: ScrubScope,
    pub generation: u64,
    pub inspected_blocks: usize,
    pub valid_blocks: usize,
    pub issue_count: usize,
    pub repairable_count: usize,
    pub read_only: bool,
}

/// Read-only result containing the exact findings that a later repair may use.
/// The generation is part of the plan, preventing a plan from being applied
/// after another writer has published a new root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrubPlan<const CAPACITY: usize> {
    pub report: ScrubReport,
    findings: [Option<ScrubFinding>; CAPACITY],
}

impl<const CAPACITY: usize> ScrubPlan<CAPACITY> {
    pub fn findings(&self) -> impl Iterator<Item = ScrubFinding> + '_ {
        self.findings.iter().flatten().copied()
    }

    pub const fn finding_count(&self) -> usize {
        self.report.issue_count
    }
}

/// Operator approval required before an operation can reclaim any block.
/// Zero is deliberately rejected, so a default value cannot authorize repair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairAuthorization(u64);

impl RepairAuthorization {
    pub const fn from_operator_confirmation(confirmation: u64) -> Option<Self> {
        if confirmation == 0 {
            None
        } else {
            Some(Self(confirmation))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairEvidence {
    pub scope: ScrubScope,
    pub block: u32,
    pub issue: ScrubIssue,
    pub before_fingerprint: u64,
    pub after_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairPreview<const CAPACITY: usize> {
    pub plan: ScrubPlan<CAPACITY>,
    pub authorization_required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairReceipt<const CAPACITY: usize> {
    pub scope: ScrubScope,
    pub generation: u64,
    pub changed_blocks: usize,
    pub evidence: [Option<RepairEvidence>; CAPACITY],
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    /// Inspect all blocks reachable by the live root and pinned snapshots.
    /// This method never mutates the filesystem and is safe to call online.
    pub fn scrub<const CAPACITY: usize>(
        &self,
        scope: ScrubScope,
    ) -> Result<ScrubPlan<CAPACITY>, Error> {
        let mut findings = [None; CAPACITY];
        let mut reachable = [false; MAX_BLOCKS];
        let mut duplicate = [false; MAX_BLOCKS];
        let mut inspected_blocks = 0;
        let mut valid_blocks = 0;
        let mut issue_count = 0;
        let mut repairable_count = 0;

        let mut root_reachable = [false; MAX_BLOCKS];
        let mut root_data_owners = [None; MAX_BLOCKS];
        self.mark_reachable_for_scrub(
            self.root,
            &mut root_reachable,
            &mut duplicate,
            &mut root_data_owners,
        );
        for (destination, source) in reachable.iter_mut().zip(root_reachable) {
            *destination |= source;
        }
        for checkpoint in self.checkpoints.iter().flatten() {
            if checkpoint.info.id.raw() == 0 || checkpoint.info.generation > self.generation {
                Self::push_finding(
                    &mut findings,
                    &mut issue_count,
                    &mut repairable_count,
                    ScrubFinding {
                        scope,
                        block: 0,
                        issue: ScrubIssue::InvalidCheckpoint,
                        before_fingerprint: EMPTY_BLOCK_FINGERPRINT,
                        repairable: false,
                    },
                )?;
            }
            let mut checkpoint_reachable = [false; MAX_BLOCKS];
            let mut checkpoint_data_owners = [None; MAX_BLOCKS];
            self.mark_reachable_for_scrub(
                checkpoint.root,
                &mut checkpoint_reachable,
                &mut duplicate,
                &mut checkpoint_data_owners,
            );
            for (destination, source) in reachable.iter_mut().zip(checkpoint_reachable) {
                *destination |= source;
            }
        }

        for (index, slot) in self.arena.slots.iter().enumerate() {
            let Some(block) = slot.block.as_ref() else {
                continue
            };
            inspected_blocks += 1;
            let id = BlockId(index as u32 + 1);
            let fingerprint = block_fingerprint(block);
            let corrupt = self.validate_scrub_block(id).is_err();
            if corrupt {
                let issue = if self.has_stale_generation(id) {
                    ScrubIssue::StaleGeneration
                } else {
                    ScrubIssue::CorruptBlock
                };
                Self::push_finding(
                    &mut findings,
                    &mut issue_count,
                    &mut repairable_count,
                    ScrubFinding {
                        scope,
                        block: id.0,
                        issue,
                        before_fingerprint: fingerprint,
                        repairable: false,
                    },
                )?;
            }
            if duplicate[index] {
                Self::push_finding(
                    &mut findings,
                    &mut issue_count,
                    &mut repairable_count,
                    ScrubFinding {
                        scope,
                        block: id.0,
                        issue: ScrubIssue::DuplicateBlock,
                        before_fingerprint: fingerprint,
                        repairable: false,
                    },
                )?;
            } else if !corrupt && !reachable[index] {
                Self::push_finding(
                    &mut findings,
                    &mut issue_count,
                    &mut repairable_count,
                    ScrubFinding {
                        scope,
                        block: id.0,
                        issue: ScrubIssue::UnreachableBlock,
                        before_fingerprint: fingerprint,
                        repairable: true,
                    },
                )?;
            } else if !corrupt {
                valid_blocks += 1;
            }
        }

        Ok(ScrubPlan {
            report: ScrubReport {
                scope,
                generation: self.generation,
                inspected_blocks,
                valid_blocks,
                issue_count,
                repairable_count,
                read_only: true,
            },
            findings,
        })
    }

    /// Produce a named preview without changing any bytes. This separate
    /// method makes the no-write default explicit at service call sites.
    pub fn repair_preview<const CAPACITY: usize>(
        &self,
        scope: ScrubScope,
    ) -> Result<RepairPreview<CAPACITY>, Error> {
        Ok(RepairPreview {
            plan: self.scrub(scope)?,
            authorization_required: true,
        })
    }

    /// Reclaim only the unreachable blocks present in a current plan. A
    /// nonzero operator confirmation and an unchanged generation are both
    /// required. Every reclaimed block gets before/after evidence.
    pub fn repair<const CAPACITY: usize>(
        &mut self,
        plan: ScrubPlan<CAPACITY>,
        authorization: Option<RepairAuthorization>,
    ) -> Result<RepairReceipt<CAPACITY>, Error> {
        if authorization.is_none() {
            return Err(Error::RepairUnauthorized)
        }
        if plan.report.generation != self.generation {
            return Err(Error::StaleRepairPlan)
        }

        let mut evidence = [None; CAPACITY];
        let mut changed_blocks = 0;
        for (index, finding) in plan.findings.iter().flatten().copied().enumerate() {
            if finding.issue != ScrubIssue::UnreachableBlock || !finding.repairable {
                continue
            }
            let Some(slot) = self.arena.slots.get_mut(finding.block as usize - 1) else {
                return Err(Error::StaleRepairPlan)
            };
            let Some(block) = slot.block.as_ref() else {
                return Err(Error::StaleRepairPlan)
            };
            if block_fingerprint(block) != finding.before_fingerprint {
                return Err(Error::StaleRepairPlan)
            }
            slot.block = None;
            evidence[index] = Some(RepairEvidence {
                scope: finding.scope,
                block: finding.block,
                issue: finding.issue,
                before_fingerprint: finding.before_fingerprint,
                after_fingerprint: EMPTY_BLOCK_FINGERPRINT,
            });
            changed_blocks += 1;
        }

        Ok(RepairReceipt {
            scope: plan.report.scope,
            generation: self.generation,
            changed_blocks,
            evidence,
        })
    }

    fn push_finding<const CAPACITY: usize>(
        findings: &mut [Option<ScrubFinding>; CAPACITY],
        issue_count: &mut usize,
        repairable_count: &mut usize,
        finding: ScrubFinding,
    ) -> Result<(), Error> {
        let slot = findings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(Error::BufferTooSmall {
                required: issue_count.saturating_add(1),
            })?;
        *slot = Some(finding);
        *issue_count += 1;
        if finding.repairable {
            *repairable_count += 1;
        }
        Ok(())
    }

    fn validate_scrub_block(&self, id: BlockId) -> Result<(), Error> {
        match self.arena.get(id)? {
            Block::Data(data) => {
                let length = usize::from(data.len);
                if length == 0
                    || length > super::DATA_BYTES
                    || checksum(&data.bytes[..length]) != data.checksum
                {
                    return Err(Error::Corrupt)
                }
                if data.next.is_some() {
                    match self.arena.get(data.next)? {
                        Block::Data(_) => {}
                        Block::Tree(_) => return Err(Error::Corrupt),
                    }
                }
            }
            Block::Tree(TreeBlock::Leaf(leaf)) => {
                let length = usize::from(leaf.len);
                if length == 0 || length > super::MAX_KEYS {
                    return Err(Error::Corrupt)
                }
                for pair in leaf.records[..length].windows(2) {
                    if pair[0].key >= pair[1].key {
                        return Err(Error::Corrupt)
                    }
                }
                for record in &leaf.records[..length] {
                    if record.key.version == 0
                        || record.key.file.len == 0
                        || usize::from(record.key.file.len) > super::MAX_PATH_BYTES
                        || record.created_at > self.generation
                    {
                        return Err(Error::Corrupt)
                    }
                    if record.deleted {
                        if record.size != 0 || record.data.is_some() {
                            return Err(Error::Corrupt)
                        }
                    } else {
                        if record.link_count == 0 {
                            return Err(Error::Corrupt)
                        }
                        match record.file_type {
                            super::FileType::Directory => {
                                if record.size != 0 || record.data.is_some() {
                                    return Err(Error::Corrupt)
                                }
                            }
                            super::FileType::Regular | super::FileType::Symlink => {
                                self.validate_scrub_data(record.data, record.size, record.checksum)?
                            }
                        }
                    }
                }
            }
            Block::Tree(TreeBlock::Branch(branch)) => {
                let length = usize::from(branch.len);
                if length == 0 || length > super::MAX_KEYS {
                    return Err(Error::Corrupt)
                }
                for pair in branch.keys[..length].windows(2) {
                    if pair[0] >= pair[1] {
                        return Err(Error::Corrupt)
                    }
                }
                for child in &branch.children[..=length] {
                    match self.arena.get(*child)? {
                        Block::Tree(_) => {}
                        Block::Data(_) => return Err(Error::Corrupt),
                    }
                }
            }
        }
        Ok(())
    }

    fn has_stale_generation(&self, id: BlockId) -> bool {
        match self.arena.get(id) {
            Ok(Block::Tree(TreeBlock::Leaf(leaf))) => leaf.records[..usize::from(leaf.len).min(super::MAX_KEYS)]
                .iter()
                .any(|record| record.created_at > self.generation),
            _ => false,
        }
    }

    fn validate_scrub_data(
        &self,
        first: BlockId,
        size: u64,
        expected_checksum: u64,
    ) -> Result<(), Error> {
        let mut id = first;
        let mut total = 0u64;
        let mut data_checksum = 0xcbf29ce484222325_u64;
        let mut seen = [false; MAX_BLOCKS];
        while id.is_some() {
            let index = id.0.checked_sub(1).map(|value| value as usize).ok_or(Error::Corrupt)?;
            if index >= MAX_BLOCKS || seen[index] {
                return Err(Error::Corrupt)
            }
            seen[index] = true;
            let Block::Data(data) = self.arena.get(id)? else {
                return Err(Error::Corrupt)
            };
            let length = usize::from(data.len);
            if length == 0
                || length > super::DATA_BYTES
                || checksum(&data.bytes[..length]) != data.checksum
            {
                return Err(Error::Corrupt)
            }
            for byte in &data.bytes[..length] {
                data_checksum ^= *byte as u64;
                data_checksum = data_checksum.wrapping_mul(0x100000001b3);
            }
            total = total.checked_add(length as u64).ok_or(Error::Corrupt)?;
            id = data.next;
        }
        if total != size || data_checksum != expected_checksum {
            return Err(Error::Corrupt)
        }
        Ok(())
    }

    fn mark_reachable_for_scrub(
        &self,
        root: BlockId,
        marked: &mut [bool; MAX_BLOCKS],
        duplicate: &mut [bool; MAX_BLOCKS],
        data_owners: &mut [Option<u64>; MAX_BLOCKS],
    ) {
        if !root.is_some() {
            return
        }
        let mut pending = [(BlockId::NONE, None); MAX_BLOCKS];
        let mut pending_len = 0;
        let Some(index) = root.0.checked_sub(1).map(|value| value as usize) else {
            return
        };
        if index >= MAX_BLOCKS {
            return
        }
        marked[index] = true;
        pending[pending_len] = (root, None);
        pending_len += 1;
        while pending_len != 0 {
            pending_len -= 1;
            let (id, owner) = pending[pending_len];
            let Ok(block) = self.arena.get(id) else {
                continue
            };
            match block {
                Block::Data(data) => self.mark_scrub_id(
                    data.next,
                    marked,
                    duplicate,
                    data_owners,
                    owner,
                    &mut pending,
                    &mut pending_len,
                ),
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..usize::from(leaf.len).min(super::MAX_KEYS)] {
                        if !record.deleted {
                            self.mark_scrub_id(
                                record.data,
                                marked,
                                duplicate,
                                data_owners,
                                Some(record.object_id),
                                &mut pending,
                                &mut pending_len,
                            )
                        }
                    }
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    for child in &branch.children[..=usize::from(branch.len).min(super::MAX_KEYS)] {
                        self.mark_scrub_id(
                            *child,
                            marked,
                            duplicate,
                            data_owners,
                            None,
                            &mut pending,
                            &mut pending_len,
                        )
                    }
                }
            }
        }
    }

    fn mark_scrub_id(
        &self,
        id: BlockId,
        marked: &mut [bool; MAX_BLOCKS],
        duplicate: &mut [bool; MAX_BLOCKS],
        data_owners: &mut [Option<u64>; MAX_BLOCKS],
        owner: Option<u64>,
        pending: &mut [(BlockId, Option<u64>); MAX_BLOCKS],
        pending_len: &mut usize,
    ) {
        let Some(index) = id.0.checked_sub(1).map(|value| value as usize) else {
            return
        };
        if index >= MAX_BLOCKS {
            return
        }
        if marked[index] {
            if owner.is_none() || data_owners[index].is_some_and(|existing| Some(existing) != owner) {
                duplicate[index] = true;
            }
            return
        }
        if *pending_len >= MAX_BLOCKS {
            return
        }
        marked[index] = true;
        data_owners[index] = owner;
        pending[*pending_len] = (id, owner);
        *pending_len += 1;
    }
}

fn block_fingerprint(block: &Block) -> u64 {
    match block {
        Block::Data(data) => {
            let length = usize::from(data.len).min(super::DATA_BYTES);
            let mut material = [0; super::DATA_BYTES + 24];
            material[..8].copy_from_slice(&u64::from(data.next.0).to_le_bytes());
            material[8..10].copy_from_slice(&data.len.to_le_bytes());
            material[10..18].copy_from_slice(&data.checksum.to_le_bytes());
            material[24..24 + length].copy_from_slice(&data.bytes[..length]);
            checksum(&material[..24 + length])
        }
        Block::Tree(tree) => checksum(&tree_fingerprint_bytes(tree)),
    }
}

fn tree_fingerprint_bytes(tree: &TreeBlock) -> [u8; super::BLOCK_SIZE] {
    let mut bytes = [0; super::BLOCK_SIZE];
    match tree {
        TreeBlock::Leaf(leaf) => {
            bytes[0] = 1;
            bytes[1] = leaf.len;
            let mut cursor = 2;
            for record in &leaf.records {
                bytes[cursor..cursor + 4].copy_from_slice(&record.key.version.to_le_bytes());
                cursor += 4;
                bytes[cursor..cursor + 8].copy_from_slice(&record.object_id.to_le_bytes());
                cursor += 8;
                bytes[cursor..cursor + 8].copy_from_slice(&record.size.to_le_bytes());
                cursor += 8;
                bytes[cursor..cursor + 8]
                    .copy_from_slice(&u64::from(record.data.0).to_le_bytes());
                cursor += 8;
                bytes[cursor..cursor + 8].copy_from_slice(&record.checksum.to_le_bytes());
                cursor += 8;
                bytes[cursor] = u8::from(record.deleted);
                cursor += 1;
                bytes[cursor] = record.file_type as u8;
                cursor += 1;
                bytes[cursor..cursor + 4].copy_from_slice(&record.link_count.to_le_bytes());
                cursor += 4;
                bytes[cursor..cursor + 2].copy_from_slice(&record.mode.to_le_bytes());
                cursor += 2;
                let name_length = usize::from(record.key.file.len).min(super::MAX_PATH_BYTES);
                bytes[cursor..cursor + name_length]
                    .copy_from_slice(&record.key.file.bytes[..name_length]);
                cursor = cursor.saturating_add(super::MAX_PATH_BYTES);
                if cursor + 8 >= bytes.len() {
                    break
                }
            }
        }
        TreeBlock::Branch(branch) => {
            bytes[0] = 2;
            bytes[1] = branch.len;
            let mut cursor = 2;
            for child in &branch.children {
                bytes[cursor..cursor + 8].copy_from_slice(&u64::from(child.0).to_le_bytes());
                cursor += 8;
            }
            for key in &branch.keys {
                bytes[cursor..cursor + 4].copy_from_slice(&key.version.to_le_bytes());
                cursor += 4;
                let name_length = usize::from(key.file.len).min(super::MAX_PATH_BYTES);
                bytes[cursor..cursor + name_length]
                    .copy_from_slice(&key.file.bytes[..name_length]);
                cursor = cursor.saturating_add(super::MAX_PATH_BYTES);
                if cursor + 8 >= bytes.len() {
                    break
                }
            }
        }
    }
    bytes
}
