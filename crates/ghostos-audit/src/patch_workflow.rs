#![forbid(unsafe_code)]

use ghostos_script::{CowSandbox, SandboxDecision, SandboxReceipt};
use ghostos_status::{IntoStatus, Status};
use ghostos_system_model::{ContentId, LogicalName};

use crate::{AuditDaemon, AuditError, AuditFinding};

pub const DEFAULT_PATCH_PLAN_CAPACITY: usize = 32;
pub const MAX_PATCH_PATH_BYTES: usize = 256;
pub const MAX_PATCH_CONTENT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchPath {
    bytes: [u8; MAX_PATCH_PATH_BYTES],
    length: u16,
}

impl PatchPath {
    pub fn new(path: &str) -> Result<Self, PatchWorkflowError> {
        if path.is_empty() || path.len() > MAX_PATCH_PATH_BYTES || !path.is_ascii() {
            return Err(PatchWorkflowError::InvalidPath);
        }

        let mut bytes = [0; MAX_PATCH_PATH_BYTES];
        bytes[..path.len()].copy_from_slice(path.as_bytes());
        Ok(Self {
            bytes,
            length: path.len() as u16,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize])
            .expect("PatchPath contains validated ASCII")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchContents {
    bytes: [u8; MAX_PATCH_CONTENT_BYTES],
    length: u16,
}

impl PatchContents {
    pub fn new(contents: &[u8]) -> Result<Self, PatchWorkflowError> {
        if contents.len() > MAX_PATCH_CONTENT_BYTES {
            return Err(PatchWorkflowError::ContentsTooLarge);
        }

        let mut bytes = [0; MAX_PATCH_CONTENT_BYTES];
        bytes[..contents.len()].copy_from_slice(contents);
        Ok(Self {
            bytes,
            length: contents.len() as u16,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

/// Typed operations an AI planner may propose. Free-form agent output never
/// reaches GhostFS directly; it must first become one of these bounded values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchAction {
    WriteFile {
        path: PatchPath,
        contents: PatchContents,
    },
    DeleteFile {
        path: PatchPath,
    },
    ReplacePackage {
        binding: LogicalName,
        expected: ContentId,
        replacement: ContentId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchStep {
    pub finding: AuditFinding,
    pub action: PatchAction,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PatchPlan<const CAPACITY: usize = DEFAULT_PATCH_PLAN_CAPACITY> {
    generated_at_us: u64,
    steps: [Option<PatchStep>; CAPACITY],
}

impl<const CAPACITY: usize> PatchPlan<CAPACITY> {
    pub const fn new(generated_at_us: u64) -> Self {
        Self {
            generated_at_us,
            steps: [None; CAPACITY],
        }
    }

    pub const fn generated_at_us(&self) -> u64 {
        self.generated_at_us
    }

    pub fn steps(&self) -> impl Iterator<Item = PatchStep> + '_ {
        self.steps.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        let mut length = 0;
        while length < CAPACITY {
            if self.steps[length].is_none() {
                return length;
            }
            length += 1;
        }
        length
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn add(
        &mut self,
        finding: AuditFinding,
        action: PatchAction,
    ) -> Result<(), PatchWorkflowError> {
        if self
            .steps()
            .any(|step| step.finding == finding && step.action == action)
        {
            return Err(PatchWorkflowError::DuplicateStep);
        }
        let slot = self
            .steps
            .iter_mut()
            .find(|step| step.is_none())
            .ok_or(PatchWorkflowError::PlanCapacity)?;
        *slot = Some(PatchStep { finding, action });
        Ok(())
    }

    pub fn write_file(
        &mut self,
        finding: AuditFinding,
        path: &str,
        contents: &[u8],
    ) -> Result<(), PatchWorkflowError> {
        self.add(
            finding,
            PatchAction::WriteFile {
                path: PatchPath::new(path)?,
                contents: PatchContents::new(contents)?,
            },
        )
    }

    pub fn delete_file(
        &mut self,
        finding: AuditFinding,
        path: &str,
    ) -> Result<(), PatchWorkflowError> {
        self.add(
            finding,
            PatchAction::DeleteFile {
                path: PatchPath::new(path)?,
            },
        )
    }

    pub fn replace_package(
        &mut self,
        finding: AuditFinding,
        binding: LogicalName,
        expected: ContentId,
        replacement: ContentId,
    ) -> Result<(), PatchWorkflowError> {
        if expected == replacement {
            return Err(PatchWorkflowError::InvalidPlan);
        }
        self.add(
            finding,
            PatchAction::ReplacePackage {
                binding,
                expected,
                replacement,
            },
        )
    }
}

/// Called once for every active audit finding. An AI adapter should only
/// produce typed plan entries through `PatchPlan::write_file`,
/// `PatchPlan::delete_file`, or `PatchPlan::replace_package`.
pub trait PatchPlanGenerator {
    fn generate<const CAPACITY: usize>(
        &mut self,
        finding: AuditFinding,
        plan: &mut PatchPlan<CAPACITY>,
    ) -> Result<(), PatchWorkflowError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchWorkflowError {
    ContentsTooLarge,
    DuplicateStep,
    EmptyPlan,
    Finding(AuditError),
    InvalidPath,
    InvalidPlan,
    PlanCapacity,
    Sandbox(ghostos_script::Error),
    UnsupportedAction,
}

impl IntoStatus for PatchWorkflowError {
    fn status(self) -> Status {
        match self {
            Self::PlanCapacity => Status::NO_SPACE,
            Self::Sandbox(error) => error.status(),
            Self::Finding(error) => error.status(),
            Self::ContentsTooLarge
            | Self::DuplicateStep
            | Self::EmptyPlan
            | Self::InvalidPath
            | Self::InvalidPlan
            | Self::UnsupportedAction => Status::INVALID_ARGUMENT,
        }
    }
}

impl From<AuditError> for PatchWorkflowError {
    fn from(error: AuditError) -> Self {
        Self::Finding(error)
    }
}

impl From<ghostos_script::Error> for PatchWorkflowError {
    fn from(error: ghostos_script::Error) -> Self {
        Self::Sandbox(error)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatchDryRunReport {
    pub generated_at_us: u64,
    pub base_generation: u64,
    pub staged_generation: u64,
    pub steps_applied: usize,
    pub operations: u32,
}

/// Applies a plan operation inside the private CoW transaction.
pub trait PatchApplier {
    fn apply<const MAX_BLOCKS: usize>(
        &mut self,
        sandbox: &mut CowSandbox<'_, MAX_BLOCKS>,
        step: PatchStep,
    ) -> Result<(), PatchWorkflowError>;
}

/// Built-in applier for plans that contain filesystem writes and deletes.
/// Package replacement entries are intentionally left for a package-aware
/// applier, which can validate the replacement against its trust store before
/// changing the active root.
pub struct FilesystemPatchApplier;

impl PatchApplier for FilesystemPatchApplier {
    fn apply<const MAX_BLOCKS: usize>(
        &mut self,
        sandbox: &mut CowSandbox<'_, MAX_BLOCKS>,
        step: PatchStep,
    ) -> Result<(), PatchWorkflowError> {
        match step.action {
            PatchAction::WriteFile { path, contents } => {
                sandbox.write(path.as_str(), contents.as_bytes())?;
            }
            PatchAction::DeleteFile { path } => {
                sandbox.delete(path.as_str())?;
            }
            PatchAction::ReplacePackage { .. } => {
                return Err(PatchWorkflowError::UnsupportedAction);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchDecision {
    Discard,
    Commit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchOutcome {
    pub report: PatchDryRunReport,
    pub receipt: SandboxReceipt,
}

impl PatchOutcome {
    pub const fn committed(&self) -> bool {
        self.receipt.committed
    }
}

/// AI-assisted patch orchestration. Plan generation is separate from
/// application, and application is always performed in a private GhostFS CoW
/// transaction until the caller explicitly commits it.
pub struct PatchWorkflow;

impl PatchWorkflow {
    pub fn generate_plan<const FINDINGS: usize, const CAPACITY: usize, Generator>(
        audit: &AuditDaemon<FINDINGS>,
        generator: &mut Generator,
        generated_at_us: u64,
    ) -> Result<PatchPlan<CAPACITY>, PatchWorkflowError>
    where
        Generator: PatchPlanGenerator,
    {
        let mut plan = PatchPlan::new(generated_at_us);
        for finding in audit.findings() {
            generator.generate(finding, &mut plan)?;
        }
        if plan.is_empty() {
            return Err(PatchWorkflowError::EmptyPlan);
        }
        Ok(plan)
    }

    pub fn prepare<'filesystem, const CAPACITY: usize, const MAX_BLOCKS: usize, Applier>(
        filesystem: &'filesystem mut ghostos_ghostfs::SynFs<MAX_BLOCKS>,
        plan: PatchPlan<CAPACITY>,
        applier: &mut Applier,
    ) -> Result<PreparedPatchPlan<'filesystem, MAX_BLOCKS>, PatchWorkflowError>
    where
        Applier: PatchApplier,
    {
        let generated_at_us = plan.generated_at_us();
        let mut sandbox = CowSandbox::new(filesystem);
        let mut steps_applied = 0;
        for step in plan.steps() {
            applier.apply(&mut sandbox, step)?;
            steps_applied += 1;
        }
        Ok(PreparedPatchPlan {
            report: PatchDryRunReport {
                generated_at_us,
                base_generation: sandbox.base_generation(),
                staged_generation: sandbox.staged_generation(),
                steps_applied,
                operations: sandbox.operations(),
            },
            sandbox,
        })
    }
}

/// A successful dry run holding the private CoW root. Dropping it discards all
/// changes. Call `commit` only after policy, review, and deployment checks pass.
pub struct PreparedPatchPlan<'filesystem, const MAX_BLOCKS: usize> {
    report: PatchDryRunReport,
    sandbox: CowSandbox<'filesystem, MAX_BLOCKS>,
}

impl<const MAX_BLOCKS: usize> PreparedPatchPlan<'_, MAX_BLOCKS> {
    pub const fn report(&self) -> PatchDryRunReport {
        self.report
    }

    pub const fn base_generation(&self) -> u64 {
        self.report.base_generation
    }

    pub fn staged_generation(&self) -> u64 {
        self.sandbox.staged_generation()
    }

    pub fn read(
        &self,
        path: &str,
        destination: &mut [u8],
    ) -> Result<ghostos_ghostfs::ReadResult, PatchWorkflowError> {
        self.sandbox.read(path, destination).map_err(Into::into)
    }

    pub fn finish(self, decision: PatchDecision) -> Result<PatchOutcome, PatchWorkflowError> {
        let receipt = self
            .sandbox
            .finish(match decision {
                PatchDecision::Discard => SandboxDecision::Discard,
                PatchDecision::Commit => SandboxDecision::Commit,
            })
            .map_err(PatchWorkflowError::Sandbox)?;
        Ok(PatchOutcome {
            report: self.report,
            receipt,
        })
    }

    pub fn commit(self) -> Result<PatchOutcome, PatchWorkflowError> {
        self.finish(PatchDecision::Commit)
    }

    pub fn discard(self) -> Result<PatchOutcome, PatchWorkflowError> {
        self.finish(PatchDecision::Discard)
    }
}
