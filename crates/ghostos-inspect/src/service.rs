use ghostos_status::Status;
use ghostos_admission::{
    AdmissionAction, AdmissionController, AdmissionOutcome, AdmissionPriority, WorkClass,
};
use ghostos_audit::ObsolescenceReport;
use ghostos_observability::{
    CachePolicyRegistry, HealthReport, SloReport, MAX_CACHE_POLICIES,
};

use crate::{
    ActivityReport, CpuReport, InspectCapability, InspectionAuthority,
    InspectionRights, MemoryReport, ProcessId, ProcessSample, StorageReport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectError {
    Admission(AdmissionOutcome),
    AccessDenied,
    Capacity,
    InvalidSample,
    NotFound,
    Source(Status),
}

impl InspectError {
    pub const fn status(self) -> Status {
        match self {
            Self::Admission(outcome) => match outcome.action {
                AdmissionAction::Dropped => Status::NO_SPACE,
                AdmissionAction::Delayed | AdmissionAction::Retried => Status::BUSY,
                AdmissionAction::Admitted => Status::INTERNAL,
            },
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::Capacity => Status::NO_SPACE,
            Self::InvalidSample => Status::INVALID_ARGUMENT,
            Self::NotFound => Status::NOT_FOUND,
            Self::Source(status) => status,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Local,
    Cluster,
}

pub trait InspectionProvider {
    fn sample_memory(&mut self, report: &mut MemoryReport) -> Result<(), Status>;
    fn sample_storage(&mut self, report: &mut StorageReport) -> Result<(), Status>;
    fn sample_cpu(&mut self, report: &mut CpuReport) -> Result<(), Status>;
    fn sample_activity(&mut self, report: &mut ActivityReport) -> Result<(), Status>;
    fn sample_obsolete(&mut self, _report: &mut ObsolescenceReport) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }
    fn sample_health(&mut self, _report: &mut HealthReport) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }
    fn sample_slo(&mut self, _report: &mut SloReport) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }
    fn sample_cache(
        &mut self,
        _report: &mut CachePolicyRegistry<MAX_CACHE_POLICIES>,
    ) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }
}

/// Publish/consume handoff useful when hardware and kernel collectors run in
/// different service loops.
pub struct TelemetryStore {
    memory: MemoryReport,
    storage: StorageReport,
    cpu: CpuReport,
    activity: ActivityReport,
    obsolete: ObsolescenceReport,
    health: HealthReport,
    slo: SloReport,
    cache: CachePolicyRegistry<MAX_CACHE_POLICIES>,
}

impl TelemetryStore {
    pub const fn new() -> Self {
        Self {
            memory: MemoryReport::new(),
            storage: StorageReport::new(),
            cpu: CpuReport::new(),
            activity: ActivityReport::new(),
            obsolete: ObsolescenceReport::new(),
            health: HealthReport::new(),
            slo: SloReport::new(0),
            cache: CachePolicyRegistry::new(),
        }
    }

    pub fn publish_memory(&mut self, report: MemoryReport) {
        self.memory = report
    }

    pub fn publish_storage(&mut self, report: StorageReport) {
        self.storage = report
    }

    pub fn publish_cpu(&mut self, report: CpuReport) {
        self.cpu = report
    }

    pub fn publish_activity(&mut self, report: ActivityReport) {
        self.activity = report
    }

    pub fn publish_obsolete(&mut self, report: ObsolescenceReport) {
        self.obsolete = report
    }

    pub fn publish_health(&mut self, report: HealthReport) {
        report.emit_audit();
        self.health = report
    }

    pub fn publish_slo(&mut self, report: SloReport) {
        self.slo = report
    }

    pub fn publish_cache(&mut self, report: CachePolicyRegistry<MAX_CACHE_POLICIES>) {
        self.cache = report
    }
}

impl Default for TelemetryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectionProvider for TelemetryStore {
    fn sample_memory(&mut self, report: &mut MemoryReport) -> Result<(), Status> {
        *report = self.memory;
        Ok(())
    }

    fn sample_storage(&mut self, report: &mut StorageReport) -> Result<(), Status> {
        *report = self.storage;
        Ok(())
    }

    fn sample_cpu(&mut self, report: &mut CpuReport) -> Result<(), Status> {
        *report = self.cpu;
        Ok(())
    }

    fn sample_activity(&mut self, report: &mut ActivityReport) -> Result<(), Status> {
        *report = self.activity;
        Ok(())
    }

    fn sample_obsolete(&mut self, report: &mut ObsolescenceReport) -> Result<(), Status> {
        *report = self.obsolete;
        Ok(())
    }

    fn sample_health(&mut self, report: &mut HealthReport) -> Result<(), Status> {
        *report = self.health;
        Ok(())
    }

    fn sample_slo(&mut self, report: &mut SloReport) -> Result<(), Status> {
        *report = self.slo;
        Ok(())
    }

    fn sample_cache(
        &mut self,
        report: &mut CachePolicyRegistry<MAX_CACHE_POLICIES>,
    ) -> Result<(), Status> {
        *report = self.cache;
        Ok(())
    }
}

pub struct InspectionService<Provider> {
    authority: InspectionAuthority,
    provider: Provider,
    admission: AdmissionController,
}

impl<Provider> InspectionService<Provider> {
    pub fn new(authority: InspectionAuthority, provider: Provider) -> Self {
        Self::new_with_admission(authority, provider, AdmissionController::default())
    }

    pub fn new_with_admission(
        authority: InspectionAuthority,
        provider: Provider,
        admission: AdmissionController,
    ) -> Self {
        Self {
            authority,
            provider,
            admission,
        }
    }

    pub const fn authority(&self) -> InspectionAuthority {
        self.authority
    }

    pub fn authority_mut(&mut self) -> &mut InspectionAuthority {
        &mut self.authority
    }

    pub const fn provider(&self) -> &Provider {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut Provider {
        &mut self.provider
    }

    pub fn into_provider(self) -> Provider {
        self.provider
    }

    pub const fn admission(&self) -> &AdmissionController {
        &self.admission
    }

    pub fn admission_mut(&mut self) -> &mut AdmissionController {
        &mut self.admission
    }
}

impl<Provider: InspectionProvider> InspectionService<Provider> {
    pub fn memory(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<MemoryReport, InspectError> {
        self.authorize(capability, InspectionRights::MEMORY, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = MemoryReport::new();
        let result = self.provider
            .sample_memory(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    report.retain_node(capability.home_node())
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn storage(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<StorageReport, InspectError> {
        self.authorize(capability, InspectionRights::STORAGE, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = StorageReport::new();
        let result = self.provider
            .sample_storage(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    report.retain_node(capability.home_node())
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn cpu(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<CpuReport, InspectError> {
        self.authorize(capability, InspectionRights::CPU, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = CpuReport::new();
        let result = self.provider
            .sample_cpu(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    report.retain_node(capability.home_node())
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn activity(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<ActivityReport, InspectError> {
        self.authorize(capability, InspectionRights::ACTIVITY, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = ActivityReport::new();
        let result = self.provider
            .sample_activity(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    report.retain_principal(capability.subject())
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn process(
        &mut self,
        capability: InspectCapability,
        process: Option<ProcessId>,
        now_us: u64,
    ) -> Result<ProcessSample, InspectError> {
        let view = if capability.rights().contains(InspectionRights::AUDIT_WORLD) {
            View::Cluster
        } else {
            View::Local
        };
        let report = self.activity(capability, view, now_us)?;
        report
            .processes()
            .find(|sample| process.is_none_or(|id| sample.id == id))
            .ok_or(InspectError::NotFound)
    }

    pub fn obsolete(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<ObsolescenceReport, InspectError> {
        self.authorize(capability, InspectionRights::OBSOLESCENCE, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = ObsolescenceReport::new();
        let result = self.provider
            .sample_obsolete(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    let node = capability.home_node();
                    let mut local = ObsolescenceReport::new();
                    local.set_sampled_at_us(report.sampled_at_us());
                    for package in report.packages().filter(|package| package.node == node) {
                        local.push(package).map_err(|_| InspectError::InvalidSample)?;
                    }
                    report = local;
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn health(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<HealthReport, InspectError> {
        self.authorize(capability, InspectionRights::HEALTH, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = HealthReport::new();
        let result = self.provider
            .sample_health(&mut report)
            .map_err(InspectError::Source)
            .and_then(|()| {
                if view == View::Local {
                    report.retain_node(capability.home_node().raw())
                }
                Ok(report)
            });
        self.finish_view(lease, result)
    }

    pub fn cache(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<CachePolicyRegistry<MAX_CACHE_POLICIES>, InspectError> {
        self.authorize(capability, InspectionRights::CACHE, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = CachePolicyRegistry::new();
        let result = self.provider
            .sample_cache(&mut report)
            .map_err(InspectError::Source)
            .map(|()| report);
        self.finish_view(lease, result)
    }

    pub fn slo(
        &mut self,
        capability: InspectCapability,
        view: View,
        now_us: u64,
    ) -> Result<SloReport, InspectError> {
        self.authorize(capability, InspectionRights::SLO, view, now_us)?;
        let lease = self.admit_view(view)?;
        let mut report = SloReport::default();
        let result = self
            .provider
            .sample_slo(&mut report)
            .map_err(InspectError::Source)
            .map(|()| report);
        self.finish_view(lease, result)
    }

    fn authorize(
        &self,
        capability: InspectCapability,
        required: InspectionRights,
        view: View,
        now_us: u64,
    ) -> Result<(), InspectError> {
        if !self.authority.authorizes(capability, required, now_us)
            || (view == View::Cluster
                && !self.authority.authorizes(
                    capability,
                    InspectionRights::AUDIT_WORLD,
                    now_us,
                ))
        {
            return Err(InspectError::AccessDenied)
        }
        Ok(())
    }

    fn admit_view(&mut self, view: View) -> Result<Option<ghostos_admission::AdmissionLease>, InspectError> {
        if view == View::Local {
            return Ok(None)
        }
        let outcome = self
            .admission
            .admit(WorkClass::RemoteDiagnostics, AdmissionPriority::Optional);
        if outcome.admitted() {
            Ok(outcome.lease())
        } else {
            Err(InspectError::Admission(outcome))
        }
    }

    fn finish_view<T>(
        &mut self,
        lease: Option<ghostos_admission::AdmissionLease>,
        result: Result<T, InspectError>,
    ) -> Result<T, InspectError> {
        if let Some(lease) = lease {
            let _ = self.admission.finish(lease);
        }
        result
    }
}
