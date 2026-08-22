#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

pub mod patch_workflow;

pub use patch_workflow::{
    DEFAULT_PATCH_PLAN_CAPACITY, FilesystemPatchApplier, MAX_PATCH_CONTENT_BYTES,
    MAX_PATCH_PATH_BYTES, PatchAction, PatchApplier, PatchContents, PatchDecision,
    PatchDryRunReport, PatchOutcome, PatchPath, PatchPlan, PatchPlanGenerator, PatchStep,
    PatchWorkflow, PatchWorkflowError, PreparedPatchPlan,
};

use ghostos_fabric::NodeId;
use ghostos_observability::{BatchController, ProducerPolicy, ScalePath, ScalePolicy};
use ghostos_pkg::PackageDaemon;
use ghostos_status::{IntoStatus, Status};
use ghostos_system_model::{ContentId, LogicalName};

pub const DEFAULT_ADVISORY_CAPACITY: usize = 512;
pub const DEFAULT_FINDING_CAPACITY: usize = 256;
pub const MAX_ADVISORY_ID_BYTES: usize = 64;
pub const DEFAULT_OBSOLETE_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvisorySource {
    RustSec,
    Osv,
    Cve,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AdvisoryId {
    bytes: [u8; MAX_ADVISORY_ID_BYTES],
    length: u8,
}

impl AdvisoryId {
    pub fn new(value: &str) -> Result<Self, AuditError> {
        if value.is_empty() || value.len() > MAX_ADVISORY_ID_BYTES || !value.is_ascii() {
            return Err(AuditError::InvalidAdvisoryId);
        }

        let mut bytes = [0; MAX_ADVISORY_ID_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).expect("ASCII advisory identifier")
    }
}

impl fmt::Debug for AdvisoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdvisoryRecord {
    pub source: AdvisorySource,
    pub id: AdvisoryId,
    pub package_hash: ContentId,
    pub severity: Severity,
    pub withdrawn: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditError {
    AdvisoryCapacity,
    DuplicateAdvisory,
    FindingCapacity,
    InvalidAdvisoryId,
    InvalidBudget,
    CoreIsolated,
    DuplicateObsolete,
    InvalidObsolete,
    ObsoleteCapacity,
}

impl IntoStatus for AuditError {
    fn status(self) -> Status {
        match self {
            Self::AdvisoryCapacity | Self::FindingCapacity => Status::NO_SPACE,
            Self::DuplicateAdvisory | Self::InvalidAdvisoryId | Self::InvalidBudget => {
                Status::INVALID_ARGUMENT
            }
            Self::CoreIsolated => Status::BUSY,
            Self::DuplicateObsolete | Self::InvalidObsolete => Status::INVALID_ARGUMENT,
            Self::ObsoleteCapacity => Status::NO_SPACE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageKind {
    Binary,
    Driver,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl PackageVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObsolescenceReason {
    Deprecated,
    Unmaintained,
    OutOfDate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObsoletePackage {
    pub node: NodeId,
    pub package: ContentId,
    pub name: LogicalName,
    pub kind: PackageKind,
    pub reason: ObsolescenceReason,
    pub installed_version: PackageVersion,
    pub latest_version: PackageVersion,
}

#[derive(Clone, Copy)]
pub struct ObsolescenceReport<const CAPACITY: usize = DEFAULT_OBSOLETE_CAPACITY> {
    sampled_at_us: u64,
    packages: [Option<ObsoletePackage>; CAPACITY],
}

impl<const CAPACITY: usize> ObsolescenceReport<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            packages: [None; CAPACITY],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn packages(&self) -> impl Iterator<Item = ObsoletePackage> + '_ {
        self.packages.iter().flatten().copied()
    }

    pub fn push(&mut self, package: ObsoletePackage) -> Result<(), AuditError> {
        validate_obsolete(package)?;
        if self.packages().any(|entry| {
            entry.node == package.node
                && entry.package == package.package
                && entry.reason == package.reason
        }) {
            return Err(AuditError::DuplicateObsolete)
        }
        let slot = self
            .packages
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuditError::ObsoleteCapacity)?;
        *slot = Some(package);
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }
}

impl<const CAPACITY: usize> Default for ObsolescenceReport<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Fixed-capacity obsolescence index populated by package metadata feed
/// adapters. The report keeps node identity so local and cluster views can be
/// filtered without trusting shell clients with unscoped package data.
pub struct ObsolescenceRegistry<const CAPACITY: usize = DEFAULT_OBSOLETE_CAPACITY> {
    report: ObsolescenceReport<CAPACITY>,
}

impl<const CAPACITY: usize> ObsolescenceRegistry<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            report: ObsolescenceReport::new(),
        }
    }

    pub fn add(&mut self, package: ObsoletePackage) -> Result<(), AuditError> {
        self.report.push(package)
    }

    pub fn packages(&self) -> impl Iterator<Item = ObsoletePackage> + '_ {
        self.report.packages()
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.report.set_sampled_at_us(sampled_at_us)
    }

    pub fn snapshot(&self) -> ObsolescenceReport<CAPACITY> {
        self.report
    }

    pub fn clear(&mut self) {
        self.report.clear()
    }
}

impl<const CAPACITY: usize> Default for ObsolescenceRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_obsolete(package: ObsoletePackage) -> Result<(), AuditError> {
    if package.name.as_str().is_empty()
        || (package.reason == ObsolescenceReason::OutOfDate
            && package.installed_version >= package.latest_version)
    {
        return Err(AuditError::InvalidObsolete)
    }
    Ok(())
}

/// Normalized, fixed-capacity advisory index.
///
/// Feed adapters parse RustSec, OSV, or CVE data outside this crate and add
/// only the package content hashes relevant to GhostOS. Withdrawn records stay
/// indexed for deterministic feed replacement but never produce findings.
pub struct AdvisoryCatalog<const CAPACITY: usize = DEFAULT_ADVISORY_CAPACITY> {
    records: [Option<AdvisoryRecord>; CAPACITY],
}

impl<const CAPACITY: usize> AdvisoryCatalog<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAPACITY],
        }
    }

    pub fn insert(
        &mut self,
        source: AdvisorySource,
        id: AdvisoryId,
        package_hash: ContentId,
        severity: Severity,
        withdrawn: bool,
    ) -> Result<(), AuditError> {
        if self.records.iter().flatten().any(|record| {
            record.source == source && record.id == id && record.package_hash == package_hash
        }) {
            return Err(AuditError::DuplicateAdvisory);
        }

        let slot = self
            .records
            .iter_mut()
            .find(|record| record.is_none())
            .ok_or(AuditError::AdvisoryCapacity)?;
        *slot = Some(AdvisoryRecord {
            source,
            id,
            package_hash,
            severity,
            withdrawn,
        });
        Ok(())
    }

    pub fn add_rustsec(
        &mut self,
        id: &str,
        package_hash: ContentId,
        severity: Severity,
        withdrawn: bool,
    ) -> Result<(), AuditError> {
        self.add(
            AdvisorySource::RustSec,
            id,
            package_hash,
            severity,
            withdrawn,
        )
    }

    pub fn add_osv(
        &mut self,
        id: &str,
        package_hash: ContentId,
        severity: Severity,
        withdrawn: bool,
    ) -> Result<(), AuditError> {
        self.add(AdvisorySource::Osv, id, package_hash, severity, withdrawn)
    }

    pub fn add_cve(
        &mut self,
        id: &str,
        package_hash: ContentId,
        severity: Severity,
        withdrawn: bool,
    ) -> Result<(), AuditError> {
        self.add(AdvisorySource::Cve, id, package_hash, severity, withdrawn)
    }

    pub const fn len(&self) -> usize {
        let mut length = 0;
        while length < CAPACITY {
            if self.records[length].is_none() {
                return length;
            }
            length += 1;
        }
        length
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn add(
        &mut self,
        source: AdvisorySource,
        id: &str,
        package_hash: ContentId,
        severity: Severity,
        withdrawn: bool,
    ) -> Result<(), AuditError> {
        self.insert(
            source,
            AdvisoryId::new(id)?,
            package_hash,
            severity,
            withdrawn,
        )
    }
}

impl<const CAPACITY: usize> Default for AdvisoryCatalog<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait AdvisoryDatabase {
    fn visit_matches(&self, package_hash: ContentId, visitor: &mut dyn FnMut(AdvisoryRecord));
}

impl<const CAPACITY: usize> AdvisoryDatabase for AdvisoryCatalog<CAPACITY> {
    fn visit_matches(&self, package_hash: ContentId, visitor: &mut dyn FnMut(AdvisoryRecord)) {
        for record in self.records.iter().flatten().copied() {
            if record.package_hash == package_hash && !record.withdrawn {
                visitor(record)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashKind {
    Package,
    Payload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditFinding {
    pub package: ContentId,
    pub matched_hash: ContentId,
    pub hash_kind: HashKind,
    pub advisory: AdvisoryRecord,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AuditReport {
    pub packages_scanned: usize,
    pub findings_added: usize,
    pub complete: bool,
    pub batches: usize,
    pub interrupts_moderated: usize,
}

/// Cooperative background scanner. One `poll` checks at most `package_budget`
/// installed packages, so advisory work cannot starve other Ring 3 services.
pub struct AuditDaemon<const FINDINGS: usize = DEFAULT_FINDING_CAPACITY> {
    findings: [Option<AuditFinding>; FINDINGS],
    cursor: usize,
    controller: BatchController,
    scale_policy: ScalePolicy,
}

impl<const FINDINGS: usize> AuditDaemon<FINDINGS> {
    pub const fn new() -> Self {
        Self {
            findings: [None; FINDINGS],
            cursor: 0,
            controller: BatchController::new(ProducerPolicy::AUDIT),
            scale_policy: ScalePolicy::for_cpu_count(1).expect("one CPU scale tier"),
        }
    }

    pub const fn scale_policy(&self) -> ScalePolicy {
        self.scale_policy
    }

    pub fn set_scale_policy(&mut self, policy: ScalePolicy) {
        self.scale_policy = policy
    }

    pub const fn accepts_on(&self, cpu: usize) -> bool {
        self.scale_policy.accepts(ScalePath::Audit, cpu)
    }

    pub fn poll_on_cpu<
        const PACKAGES: usize,
        const KEYS: usize,
        Database: AdvisoryDatabase,
    >(
        &mut self,
        cpu: usize,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        database: &Database,
        package_budget: usize,
    ) -> Result<AuditReport, AuditError> {
        if !self.accepts_on(cpu) {
            return Err(AuditError::CoreIsolated)
        }
        self.poll(packages, database, package_budget)
    }

    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn findings(&self) -> impl Iterator<Item = AuditFinding> + '_ {
        self.findings.iter().flatten().copied()
    }

    pub fn clear(&mut self) {
        self.findings.fill(None);
        self.cursor = 0;
    }

    pub fn poll<const PACKAGES: usize, const KEYS: usize, Database: AdvisoryDatabase>(
        &mut self,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        database: &Database,
        package_budget: usize,
    ) -> Result<AuditReport, AuditError> {
        if package_budget == 0 {
            return Err(AuditError::InvalidBudget);
        }

        if self.cursor == 0 {
            self.findings.fill(None)
        }

        let decision = self.controller.plan(package_budget, 1, 0, false);
        let mut report = AuditReport::default();
        while report.packages_scanned < decision.count {
            let Some(manifest) = packages.manifests().nth(self.cursor).copied() else {
                self.cursor = 0;
                report.complete = true;
                break;
            };
            self.cursor += 1;
            report.packages_scanned += 1;

            let before = self.findings().count();
            self.visit_hash(
                database,
                manifest.content,
                manifest.content,
                HashKind::Package,
            )?;
            if manifest.payload != manifest.content {
                self.visit_hash(
                    database,
                    manifest.content,
                    manifest.payload,
                    HashKind::Payload,
                )?;
            }
            report.findings_added += self.findings().count() - before;
        }
        if report.packages_scanned != 0 {
            report.batches = 1;
            report.interrupts_moderated = usize::from(decision.interrupt);
        }
        Ok(report)
    }

    fn visit_hash<Database: AdvisoryDatabase>(
        &mut self,
        database: &Database,
        package: ContentId,
        matched_hash: ContentId,
        hash_kind: HashKind,
    ) -> Result<(), AuditError> {
        let mut overflow = false;
        database.visit_matches(matched_hash, &mut |advisory| {
            if self
                .findings
                .iter()
                .flatten()
                .any(|finding| finding.package == package && finding.advisory == advisory)
            {
                return;
            }
            if let Some(slot) = self.findings.iter_mut().find(|finding| finding.is_none()) {
                *slot = Some(AuditFinding {
                    package,
                    matched_hash,
                    hash_kind,
                    advisory,
                })
            } else {
                overflow = true
            }
        });
        if overflow {
            Err(AuditError::FindingCapacity)
        } else {
            Ok(())
        }
    }
}

impl<const FINDINGS: usize> Default for AuditDaemon<FINDINGS> {
    fn default() -> Self {
        Self::new()
    }
}
