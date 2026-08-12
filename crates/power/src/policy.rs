use crate::{ThermalAction, ThermalManager, ThermalReading, ThermalTripPoints};

pub const MAX_POWER_CLUSTERS: usize = 8;
pub const MAX_POWER_DEVICES: usize = 32;
pub const MAX_POWER_CPUS: usize = 128;
pub const MAX_FREQUENCY_POINTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ProcessorSet([u64; 2]);

impl ProcessorSet {
    pub const EMPTY: Self = Self([0; 2]);

    pub const fn all() -> Self {
        Self([u64::MAX; 2])
    }

    pub const fn from_cpu(cpu: u16) -> Self {
        let raw = cpu as usize;
        if raw >= MAX_POWER_CPUS {
            Self::EMPTY
        } else if raw < 64 {
            Self([1_u64 << raw, 0])
        } else {
            Self([0, 1_u64 << (raw - 64)])
        }
    }

    pub const fn from_words(low: u64, high: u64) -> Self {
        Self([low, high])
    }

    pub const fn raw_words(self) -> [u64; 2] {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0
    }

    pub const fn contains(self, cpu: u16) -> bool {
        let raw = cpu as usize;
        raw < MAX_POWER_CPUS && self.0[raw / 64] & (1_u64 << (raw % 64)) != 0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0[0] & other.0[0] != 0 || self.0[1] & other.0[1] != 0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self([self.0[0] & other.0[0], self.0[1] & other.0[1]])
    }

    pub const fn first(self) -> Option<u16> {
        if self.0[0] != 0 {
            Some(self.0[0].trailing_zeros() as u16)
        } else if self.0[1] != 0 {
            Some(64 + self.0[1].trailing_zeros() as u16)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuIdleState {
    C0,
    C1,
    C2,
    C3,
}

impl CpuIdleState {
    pub const fn exit_latency_us(self) -> u64 {
        match self {
            Self::C0 => 0,
            Self::C1 => 1,
            Self::C2 => 10,
            Self::C3 => 100,
        }
    }

    pub const fn power_mw(self) -> u32 {
        match self {
            Self::C0 => 1_000,
            Self::C1 => 400,
            Self::C2 => 100,
            Self::C3 => 20,
        }
    }

    pub const fn rank(self) -> u8 {
        match self {
            Self::C0 => 0,
            Self::C1 => 1,
            Self::C2 => 2,
            Self::C3 => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdleRequest {
    pub now_us: u64,
    pub next_wake_us: u64,
    pub latency_budget_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkloadClass {
    LatencyCritical = 1,
    Balanced = 2,
    Throughput = 3,
    Background = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadRequest {
    pub affinity: ProcessorSet,
    pub class: WorkloadClass,
    pub runnable: u16,
    pub latency_budget_us: u64,
    pub preferred_cluster: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PowerClusterConfig {
    pub id: u8,
    pub cpus: ProcessorSet,
    pub min_frequency_khz: u32,
    pub max_frequency_khz: u32,
    pub idle_power_mw: u32,
}

impl PowerClusterConfig {
    pub const fn new(
        id: u8,
        cpus: ProcessorSet,
        min_frequency_khz: u32,
        max_frequency_khz: u32,
        idle_power_mw: u32,
    ) -> Self {
        Self {
            id,
            cpus,
            min_frequency_khz,
            max_frequency_khz,
            idle_power_mw,
        }
    }

    const fn valid(self) -> bool {
        self.id != u8::MAX
            && !self.cpus.is_empty()
            && self.min_frequency_khz != 0
            && self.min_frequency_khz <= self.max_frequency_khz
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementDecision {
    pub cluster_id: u8,
    pub cpu: u16,
    pub cpus: ProcessorSet,
    pub latency_budget_met: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrequencyDecision {
    pub cluster_id: u8,
    pub frequency_khz: u32,
    pub throttle_percent: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevicePowerState {
    Active,
    RuntimeIdle,
    Suspended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DevicePowerConfig {
    pub id: u16,
    pub idle_after_us: u64,
    pub suspend_after_us: u64,
    pub wake_latency_us: u64,
}

impl DevicePowerConfig {
    const fn valid(self) -> bool {
        self.id != u16::MAX
            && self.idle_after_us <= self.suspend_after_us
            && self.suspend_after_us != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PowerBudget {
    pub max_latency_us: u64,
    pub max_temperature_deci_kelvin: Option<u32>,
}

impl PowerBudget {
    pub const DEFAULT: Self = Self {
        max_latency_us: 1_000,
        max_temperature_deci_kelvin: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct ThermalRecoveryMetrics {
    pub throttle_events: u64,
    pub recoveries: u64,
    pub last_recovery_us: u64,
    pub worst_recovery_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct PowerMetrics {
    pub energy_uj: u64,
    pub work_units: u64,
    pub latency_samples: u64,
    pub latency_violations: u64,
    pub thermal: ThermalRecoveryMetrics,
}

impl PowerMetrics {
    pub fn record_energy(&mut self, energy_uj: u64) {
        self.energy_uj = self.energy_uj.saturating_add(energy_uj)
    }

    pub fn record_work(&mut self, work_units: u64) {
        self.work_units = self.work_units.saturating_add(work_units)
    }

    pub fn record_latency(&mut self, latency_us: u64, budget_us: u64) {
        self.latency_samples = self.latency_samples.saturating_add(1);
        if latency_us > budget_us {
            self.latency_violations = self.latency_violations.saturating_add(1)
        }
    }

    pub const fn performance_per_watt(self) -> Option<u64> {
        if self.energy_uj == 0 {
            None
        } else {
            Some(self.work_units.saturating_mul(1_000_000) / self.energy_uj)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerPolicyError {
    Capacity,
    InvalidCluster,
    InvalidDevice,
    NoCluster,
    NoCpu,
    InvalidThermalPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClusterState {
    config: PowerClusterConfig,
    load_percent: u8,
    temperature_deci_kelvin: u32,
    throttle_percent: u8,
    valid: bool,
}

impl ClusterState {
    const EMPTY: Self = Self {
        config: PowerClusterConfig::new(
            u8::MAX,
            ProcessorSet::EMPTY,
            1,
            1,
            0,
        ),
        load_percent: 0,
        temperature_deci_kelvin: 0,
        throttle_percent: 0,
        valid: false,
    };

    fn frequency(self) -> FrequencyDecision {
        let range = self
            .config
            .max_frequency_khz
            .saturating_sub(self.config.min_frequency_khz);
        let requested = self.config.min_frequency_khz
            .saturating_add(range.saturating_mul(self.load_percent as u32) / 100);
        let allowed_range = range.saturating_mul(100_u32.saturating_sub(self.throttle_percent as u32)) / 100;
        FrequencyDecision {
            cluster_id: self.config.id,
            frequency_khz: self.config.min_frequency_khz.saturating_add(requested
                .saturating_sub(self.config.min_frequency_khz)
                .min(allowed_range)),
            throttle_percent: self.throttle_percent,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeviceState {
    config: DevicePowerConfig,
    last_active_us: u64,
    state: DevicePowerState,
    valid: bool,
}

impl DeviceState {
    const EMPTY: Self = Self {
        config: DevicePowerConfig {
            id: u16::MAX,
            idle_after_us: 0,
            suspend_after_us: 1,
            wake_latency_us: 0,
        },
        last_active_us: 0,
        state: DevicePowerState::Suspended,
        valid: false,
    };
}

pub trait PowerPolicyIo {
    type Error;

    fn set_cpu_idle_state(&mut self, cpu: u16, state: CpuIdleState) -> Result<(), Self::Error>;
    fn set_cluster_frequency(&mut self, cluster: u8, frequency_khz: u32) -> Result<(), Self::Error>;
    fn set_device_power(&mut self, device: u16, state: DevicePowerState) -> Result<(), Self::Error>;
}

pub struct PowerPolicy {
    clusters: [ClusterState; MAX_POWER_CLUSTERS],
    devices: [DeviceState; MAX_POWER_DEVICES],
    idle_states: [CpuIdleState; MAX_POWER_CPUS],
    thermal: Option<ThermalManager>,
    budget: PowerBudget,
    metrics: PowerMetrics,
    thermal_started_us: Option<u64>,
}

impl PowerPolicy {
    pub const fn new() -> Self {
        let default = ClusterState {
            config: PowerClusterConfig::new(
                0,
                ProcessorSet::all(),
                800_000,
                3_000_000,
                200,
            ),
            load_percent: 0,
            temperature_deci_kelvin: 0,
            throttle_percent: 0,
            valid: true,
        };
        let mut clusters = [ClusterState::EMPTY; MAX_POWER_CLUSTERS];
        clusters[0] = default;
        Self {
            clusters,
            devices: [DeviceState::EMPTY; MAX_POWER_DEVICES],
            idle_states: [CpuIdleState::C0; MAX_POWER_CPUS],
            thermal: None,
            budget: PowerBudget::DEFAULT,
            metrics: PowerMetrics {
                energy_uj: 0,
                work_units: 0,
                latency_samples: 0,
                latency_violations: 0,
                thermal: ThermalRecoveryMetrics {
                    throttle_events: 0,
                    recoveries: 0,
                    last_recovery_us: 0,
                    worst_recovery_us: 0,
                },
            },
            thermal_started_us: None,
        }
    }

    pub const fn budget(&self) -> PowerBudget {
        self.budget
    }

    pub fn set_budget(&mut self, budget: PowerBudget) {
        self.budget = budget
    }

    pub const fn metrics(&self) -> PowerMetrics {
        self.metrics
    }

    pub fn add_cluster(&mut self, config: PowerClusterConfig) -> Result<(), PowerPolicyError> {
        if !config.valid() {
            return Err(PowerPolicyError::InvalidCluster)
        }
        if self
            .clusters
            .iter()
            .any(|cluster| cluster.valid && cluster.config.id == config.id)
        {
            return Err(PowerPolicyError::InvalidCluster)
        }
        let slot = self
            .clusters
            .iter_mut()
            .find(|cluster| !cluster.valid)
            .ok_or(PowerPolicyError::Capacity)?;
        *slot = ClusterState {
            config,
            load_percent: 0,
            temperature_deci_kelvin: 0,
            throttle_percent: 0,
            valid: true,
        };
        Ok(())
    }

    pub fn replace_clusters(&mut self, configs: &[PowerClusterConfig]) -> Result<(), PowerPolicyError> {
        if configs.is_empty() || configs.len() > MAX_POWER_CLUSTERS {
            return Err(PowerPolicyError::Capacity)
        }
        let mut replacement = [ClusterState::EMPTY; MAX_POWER_CLUSTERS];
        for (index, config) in configs.iter().copied().enumerate() {
            if !config.valid()
                || replacement[..index]
                    .iter()
                    .any(|cluster| cluster.config.id == config.id)
            {
                return Err(PowerPolicyError::InvalidCluster)
            }
            replacement[index] = ClusterState {
                config,
                load_percent: 0,
                temperature_deci_kelvin: 0,
                throttle_percent: 0,
                valid: true,
            };
        }
        self.clusters = replacement;
        Ok(())
    }

    pub fn set_cluster_load(&mut self, cluster_id: u8, load_percent: u8) -> Result<(), PowerPolicyError> {
        let cluster = self.cluster_mut(cluster_id)?;
        cluster.load_percent = load_percent.min(100);
        Ok(())
    }

    pub fn set_cluster_temperature(
        &mut self,
        cluster_id: u8,
        temperature_deci_kelvin: u32,
    ) -> Result<(), PowerPolicyError> {
        let cluster = self.cluster_mut(cluster_id)?;
        cluster.temperature_deci_kelvin = temperature_deci_kelvin;
        Ok(())
    }

    pub fn frequency_decision(
        &self,
        cluster_id: u8,
    ) -> Result<FrequencyDecision, PowerPolicyError> {
        self.clusters
            .iter()
            .find(|cluster| cluster.valid && cluster.config.id == cluster_id)
            .map(|cluster| cluster.frequency())
            .ok_or(PowerPolicyError::InvalidCluster)
    }

    pub fn place(&mut self, request: WorkloadRequest) -> Result<PlacementDecision, PowerPolicyError> {
        let mut selected = None;
        for (index, cluster) in self.clusters.iter().enumerate() {
            if !cluster.valid
                || cluster.throttle_percent == 100
                || !cluster.config.cpus.intersects(request.affinity)
                || request.preferred_cluster.is_some_and(|id| id != cluster.config.id)
            {
                continue
            }
            let latency_penalty = if request.class == WorkloadClass::LatencyCritical {
                cluster.throttle_percent as u32 * 10
            } else {
                0
            };
            let score = cluster.load_percent as u32 * 100
                + cluster.config.idle_power_mw
                + latency_penalty;
            if selected.is_none_or(|(_, best_score)| score < best_score) {
                selected = Some((index, score))
            }
        }
        if selected.is_none() && request.preferred_cluster.is_some() {
            return self.place(WorkloadRequest {
                preferred_cluster: None,
                ..request
            })
        }
        let (index, _) = selected.ok_or(PowerPolicyError::NoCluster)?;
        let cluster = self.clusters[index];
        let cpus = cluster.config.cpus.intersection(request.affinity);
        let cpu = cpus.first().ok_or(PowerPolicyError::NoCpu)?;
        let latency_budget_met = request.latency_budget_us == 0
            || cluster.throttle_percent < 75
            || request.class != WorkloadClass::LatencyCritical;
        if !latency_budget_met {
            self.metrics.latency_violations = self.metrics.latency_violations.saturating_add(1)
        }
        Ok(PlacementDecision {
            cluster_id: cluster.config.id,
            cpu,
            cpus,
            latency_budget_met,
        })
    }

    pub fn idle_state(&self, request: IdleRequest) -> CpuIdleState {
        let available_us = request.next_wake_us.saturating_sub(request.now_us);
        let limit_us = available_us.min(request.latency_budget_us);
        [CpuIdleState::C3, CpuIdleState::C2, CpuIdleState::C1]
            .into_iter()
            .find(|state| state.exit_latency_us() <= limit_us)
            .unwrap_or(CpuIdleState::C0)
    }

    pub fn request_idle(&mut self, cpu: u16, request: IdleRequest) -> CpuIdleState {
        let state = self.idle_state(request);
        if (cpu as usize) < MAX_POWER_CPUS {
            self.idle_states[cpu as usize] = state
        }
        state
    }

    pub fn idle_state_for_cpu(&self, cpu: u16) -> CpuIdleState {
        self.idle_states
            .get(cpu as usize)
            .copied()
            .unwrap_or(CpuIdleState::C0)
    }

    pub fn configure_thermal(
        &mut self,
        trips: ThermalTripPoints,
        hysteresis_deci_kelvin: u32,
    ) -> Result<(), PowerPolicyError> {
        self.thermal = Some(
            ThermalManager::new(trips, hysteresis_deci_kelvin)
                .map_err(|_| PowerPolicyError::InvalidThermalPolicy)?,
        );
        Ok(())
    }

    pub fn update_thermal(&mut self, reading: ThermalReading) -> ThermalAction {
        let Some(thermal) = self.thermal.as_mut() else {
            return ThermalAction::Normal
        };
        let previous = thermal.action();
        let action = thermal.update(reading);
        let throttle = match action {
            ThermalAction::Normal => 0,
            ThermalAction::Throttle { percent } => percent,
            ThermalAction::EmergencyShutdown => 100,
        };
        for cluster in &mut self.clusters {
            if cluster.valid {
                cluster.throttle_percent = throttle
            }
        }
        if matches!(action, ThermalAction::Throttle { .. } | ThermalAction::EmergencyShutdown)
            && matches!(previous, ThermalAction::Normal)
        {
            self.metrics.thermal.throttle_events =
                self.metrics.thermal.throttle_events.saturating_add(1);
            self.thermal_started_us = Some(reading.timestamp_us)
        }
        if matches!(action, ThermalAction::Normal)
            && !matches!(previous, ThermalAction::Normal)
        {
            if let Some(started) = self.thermal_started_us.take() {
                let recovery = reading.timestamp_us.saturating_sub(started);
                self.metrics.thermal.recoveries = self.metrics.thermal.recoveries.saturating_add(1);
                self.metrics.thermal.last_recovery_us = recovery;
                self.metrics.thermal.worst_recovery_us =
                    self.metrics.thermal.worst_recovery_us.max(recovery)
            }
        }
        action
    }

    pub fn add_device(&mut self, config: DevicePowerConfig) -> Result<(), PowerPolicyError> {
        if !config.valid()
            || self
                .devices
                .iter()
                .any(|device| device.valid && device.config.id == config.id)
        {
            return Err(PowerPolicyError::InvalidDevice)
        }
        let slot = self
            .devices
            .iter_mut()
            .find(|device| !device.valid)
            .ok_or(PowerPolicyError::Capacity)?;
        *slot = DeviceState {
            config,
            last_active_us: 0,
            state: DevicePowerState::Active,
            valid: true,
        };
        Ok(())
    }

    pub fn note_device_activity(&mut self, device_id: u16, now_us: u64) -> Result<(), PowerPolicyError> {
        let device = self.device_mut(device_id)?;
        device.last_active_us = now_us;
        device.state = DevicePowerState::Active;
        Ok(())
    }

    pub fn device_state(
        &mut self,
        device_id: u16,
        now_us: u64,
    ) -> Result<DevicePowerState, PowerPolicyError> {
        let device = self.device_mut(device_id)?;
        let idle_for = now_us.saturating_sub(device.last_active_us);
        device.state = if idle_for >= device.config.suspend_after_us {
            DevicePowerState::Suspended
        } else if idle_for >= device.config.idle_after_us {
            DevicePowerState::RuntimeIdle
        } else {
            DevicePowerState::Active
        };
        Ok(device.state)
    }

    pub fn apply<I: PowerPolicyIo>(&mut self, now_us: u64, io: &mut I) -> Result<(), I::Error> {
        for cluster in self.clusters.iter().copied().filter(|cluster| cluster.valid) {
            let decision = cluster.frequency();
            io.set_cluster_frequency(decision.cluster_id, decision.frequency_khz)?;
        }
        let devices = self.devices;
        for device in devices.into_iter().filter(|device| device.valid) {
            let state = self.device_state(device.config.id, now_us).unwrap_or(DevicePowerState::Active);
            io.set_device_power(device.config.id, state)?;
        }
        Ok(())
    }

    pub fn apply_idle<I: PowerPolicyIo>(&mut self, cpu: u16, request: IdleRequest, io: &mut I) -> Result<CpuIdleState, I::Error> {
        let state = self.request_idle(cpu, request);
        io.set_cpu_idle_state(cpu, state)?;
        Ok(state)
    }

    fn cluster_mut(&mut self, cluster_id: u8) -> Result<&mut ClusterState, PowerPolicyError> {
        self.clusters
            .iter_mut()
            .find(|cluster| cluster.valid && cluster.config.id == cluster_id)
            .ok_or(PowerPolicyError::InvalidCluster)
    }

    fn device_mut(&mut self, device_id: u16) -> Result<&mut DeviceState, PowerPolicyError> {
        self.devices
            .iter_mut()
            .find(|device| device.valid && device.config.id == device_id)
            .ok_or(PowerPolicyError::InvalidDevice)
    }
}

impl Default for PowerPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_selection_respects_latency_budget() {
        let policy = PowerPolicy::new();
        assert_eq!(
            policy.idle_state(IdleRequest {
                now_us: 100,
                next_wake_us: 200,
                latency_budget_us: 50,
            }),
            CpuIdleState::C2
        );
        assert_eq!(
            policy.idle_state(IdleRequest {
                now_us: 100,
                next_wake_us: 150,
                latency_budget_us: 5,
            }),
            CpuIdleState::C1
        );
    }

    #[test]
    fn thermal_throttle_recovers_and_records_duration() {
        let mut policy = PowerPolicy::new();
        policy
            .configure_thermal(
                ThermalTripPoints {
                    passive_deci_kelvin: Some(3300),
                    hot_deci_kelvin: Some(3400),
                    critical_deci_kelvin: Some(3600),
                },
                50,
            )
            .unwrap();
        assert!(matches!(
            policy.update_thermal(ThermalReading {
                temperature_deci_kelvin: 3400,
                timestamp_us: 10,
            }),
            ThermalAction::Throttle { .. }
        ));
        assert_eq!(
            policy.update_thermal(ThermalReading {
                temperature_deci_kelvin: 3200,
                timestamp_us: 75,
            }),
            ThermalAction::Normal
        );
        assert_eq!(policy.metrics().thermal.last_recovery_us, 65);
    }

    #[test]
    fn placement_keeps_latency_work_on_an_allowed_cluster() {
        let mut policy = PowerPolicy::new();
        policy
            .replace_clusters(&[
                PowerClusterConfig::new(1, ProcessorSet::from_cpu(0), 800_000, 3_000_000, 300),
                PowerClusterConfig::new(2, ProcessorSet::from_cpu(1), 1_000_000, 3_500_000, 500),
            ])
            .unwrap();
        policy.set_cluster_load(1, 90).unwrap();
        let placement = policy
            .place(WorkloadRequest {
                affinity: ProcessorSet::from_words(0b11, 0),
                class: WorkloadClass::LatencyCritical,
                runnable: 1,
                latency_budget_us: 100,
                preferred_cluster: None,
            })
            .unwrap();
        assert_eq!(placement.cluster_id, 2);
        assert_eq!(placement.cpu, 1);
        assert!(placement.latency_budget_met);
    }
}
