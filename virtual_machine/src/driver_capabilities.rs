//! Bounded driver capability discovery and fallback reporting.
//!
//! The VM owns the guest device models, so a missing host facility must not
//! change guest-visible behavior.  Discovery records the selected path and
//! the fallback semantics together; operators can see why a request is
//! running on a slower or less capable implementation.

use crate::firmware::FirmwareMode;
use crate::hardware_acceleration::{HardwareAcceleration, HardwareAccelerationStatus};
use crate::net::NetworkBackendConfig;

pub const DRIVER_CAPABILITY_COUNT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverCapabilityKind {
    Acceleration,
    Storage,
    NicOffload,
    Gpu,
    Firmware,
    Timer,
}

impl DriverCapabilityKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Acceleration => "acceleration",
            Self::Storage => "storage",
            Self::NicOffload => "nic-offload",
            Self::Gpu => "gpu",
            Self::Firmware => "firmware",
            Self::Timer => "platform-timer",
        }
    }
}

impl std::fmt::Display for DriverCapabilityKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One discovered feature and the path selected for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverCapability {
    pub kind: DriverCapabilityKind,
    pub feature: &'static str,
    pub available: bool,
    pub selected: &'static str,
    pub fallback: &'static str,
    pub semantics: &'static str,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct CCapability {
    kind: u32,
    available: bool,
    feature: *const std::ffi::c_char,
    selected: *const std::ffi::c_char,
    fallback: *const std::ffi::c_char,
    semantics: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn ghostos_vm_driver_discover(uefi: bool, network: u32, requested_native: bool,
        native_execution: bool, output: *mut CCapability) -> bool;
}

// The C report only returns immutable static UTF-8 string literals.
fn report_text(text: *const std::ffi::c_char) -> &'static str {
    unsafe { std::ffi::CStr::from_ptr(text) }.to_str().expect("C driver report UTF-8")
}

/// Complete, fixed-size capability report for one VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverCapabilityReport {
    entries: [DriverCapability; DRIVER_CAPABILITY_COUNT],
}

impl DriverCapabilityReport {
    /// Probe host-backed facilities and choose only paths with stable guest
    /// semantics.  The software device models are the deliberate fallback
    /// when optional acceleration or offload facilities are absent.
    pub fn discover(
        firmware: FirmwareMode,
        network: &NetworkBackendConfig,
        acceleration: &HardwareAccelerationStatus,
    ) -> Self {
        let network = match network {
            NetworkBackendConfig::Deterministic | NetworkBackendConfig::DeterministicShared { .. } => 0,
            NetworkBackendConfig::UserNat { .. } => 1,
            NetworkBackendConfig::Bridged { .. } => 2,
        };
        let mut output = std::mem::MaybeUninit::<[CCapability; DRIVER_CAPABILITY_COUNT]>::uninit();
        let discovered = unsafe {
            ghostos_vm_driver_discover(firmware == FirmwareMode::Uefi, network,
                acceleration.requested.is_native(), acceleration.execution_backend.is_native(),
                output.as_mut_ptr().cast())
        };
        assert!(discovered, "invalid C driver discovery inputs");
        let entries = unsafe { output.assume_init() }.map(|entry| DriverCapability {
            kind: match entry.kind {
                0 => DriverCapabilityKind::Acceleration,
                1 => DriverCapabilityKind::Storage,
                2 => DriverCapabilityKind::NicOffload,
                3 => DriverCapabilityKind::Gpu,
                4 => DriverCapabilityKind::Firmware,
                5 => DriverCapabilityKind::Timer,
                _ => panic!("invalid C driver capability kind"),
            },
            feature: report_text(entry.feature),
            available: entry.available,
            selected: report_text(entry.selected),
            fallback: report_text(entry.fallback),
            semantics: report_text(entry.semantics),
        });
        Self { entries }
    }

    pub const fn entries(&self) -> &[DriverCapability; DRIVER_CAPABILITY_COUNT] {
        &self.entries
    }

    pub fn iter(&self) -> impl Iterator<Item = &DriverCapability> {
        self.entries.iter()
    }
}

impl Default for DriverCapabilityReport {
    fn default() -> Self {
        let acceleration = HardwareAccelerationStatus {
            requested: HardwareAcceleration::Software,
            active: HardwareAcceleration::Software,
            execution_backend: HardwareAcceleration::Software,
            fallback: false,
            fallback_behavior: crate::hardware_acceleration::HardwareAccelerationFallback::None,
            supported_features: &[],
            limitations: &[],
            attempts: Vec::new(),
            description: "portable software execution",
        };
        Self::discover(FirmwareMode::Bios, &NetworkBackendConfig::Deterministic, &acceleration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware_acceleration::HardwareAccelerationFallback;

    fn status(requested: HardwareAcceleration) -> HardwareAccelerationStatus {
        HardwareAccelerationStatus {
            requested,
            active: HardwareAcceleration::Software,
            execution_backend: HardwareAcceleration::Software,
            fallback: requested.is_native(),
            fallback_behavior: if requested.is_native() {
                HardwareAccelerationFallback::NoNativeBackend
            } else {
                HardwareAccelerationFallback::None
            },
            supported_features: &[],
            limitations: &[],
            attempts: Vec::new(),
            description: "portable software execution",
        }
    }

    #[test]
    fn missing_native_execution_reports_portable_fallback() {
        let report = DriverCapabilityReport::discover(
            FirmwareMode::Bios,
            &NetworkBackendConfig::Deterministic,
            &status(HardwareAcceleration::Kvm),
        );
        let acceleration = report
            .iter()
            .find(|entry| entry.feature == "native-guest-execution")
            .expect("acceleration capability");
        assert!(!acceleration.available);
        assert_eq!(acceleration.selected, "portable-cpu-execution");
        assert_eq!(acceleration.fallback, "portable-cpu-execution");
        assert!(acceleration.semantics.contains("equivalent"));
    }

    #[test]
    fn absent_optional_features_keep_explicit_stable_fallbacks() {
        let report = DriverCapabilityReport::default();
        for feature in [
            "storage-discard",
            "async-storage-queue",
            "hardware-offloads",
            "host-gpu-acceleration",
        ] {
            let capability = report
                .iter()
                .find(|entry| entry.feature == feature)
                .expect("optional capability");
            assert!(!capability.available);
            assert_eq!(capability.selected, capability.fallback);
            assert!(!capability.semantics.is_empty());
        }
    }

    #[test]
    fn firmware_and_timer_choices_are_visible() {
        let report = DriverCapabilityReport::discover(
            FirmwareMode::Uefi,
            &NetworkBackendConfig::Deterministic,
            &status(HardwareAcceleration::Software),
        );
        let runtime = report
            .iter()
            .find(|entry| entry.feature == "uefi-runtime-services")
            .expect("runtime capability");
        let timer = report
            .iter()
            .find(|entry| entry.feature == "platform-timer-source")
            .expect("timer capability");
        assert!(runtime.available);
        assert_eq!(runtime.selected, "uefi-runtime-services");
        assert_eq!(timer.selected, "shared-monotonic-clock");
    }
}
