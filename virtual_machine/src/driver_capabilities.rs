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

impl DriverCapability {
    const fn new(
        kind: DriverCapabilityKind,
        feature: &'static str,
        available: bool,
        selected: &'static str,
        fallback: &'static str,
        semantics: &'static str,
    ) -> Self {
        Self {
            kind,
            feature,
            available,
            selected,
            fallback,
            semantics,
        }
    }
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
        let native_acceleration = acceleration.execution_backend.is_native();
        let network_path = match network {
            NetworkBackendConfig::Deterministic | NetworkBackendConfig::DeterministicShared { .. } => {
                "deterministic-software-ethernet"
            }
            NetworkBackendConfig::UserNat { .. } => "userspace-nat-ethernet",
            NetworkBackendConfig::Bridged { .. } => "host-bridged-ethernet",
        };
        let firmware_path = match firmware {
            FirmwareMode::Bios => "legacy-bios-services",
            FirmwareMode::Uefi => "uefi-boot-services",
        };

        Self {
            entries: [
                DriverCapability::new(
                    DriverCapabilityKind::Acceleration,
                    "native-guest-execution",
                    !acceleration.requested.is_native() || native_acceleration,
                    if native_acceleration {
                        "native-guest-execution"
                    } else {
                        "portable-cpu-execution"
                    },
                    if acceleration.requested.is_native() && !native_acceleration {
                        "portable-cpu-execution"
                    } else {
                        "none"
                    },
                    "instruction results, interrupts, device effects, replay, and snapshots stay equivalent",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Storage,
                    "controller-flush",
                    true,
                    "controller-flush",
                    "ordered-image-flush",
                    "flush completion keeps the existing durability boundary",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Storage,
                    "storage-discard",
                    false,
                    "unsupported-status",
                    "unsupported-status",
                    "a missing discard operation is rejected explicitly; readable data is not changed",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Storage,
                    "async-storage-queue",
                    false,
                    "bounded-synchronous-image-io",
                    "bounded-synchronous-image-io",
                    "request ordering and completion status remain deterministic",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::NicOffload,
                    "hardware-offloads",
                    false,
                    "software-packet-processing",
                    "software-packet-processing",
                    "wire bytes, checksum behavior, delivery order, and queue limits remain unchanged",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::NicOffload,
                    "network-backend",
                    true,
                    network_path,
                    "deterministic-software-ethernet",
                    "packet framing and guest-visible link state remain explicit",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Gpu,
                    "host-gpu-acceleration",
                    false,
                    "software-vga-vesa-rendering",
                    "software-vga-vesa-rendering",
                    "text and framebuffer pixels are rendered by the VM device model",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Firmware,
                    "firmware-services",
                    true,
                    firmware_path,
                    "guest-visible-unsupported-status",
                    "unsupported firmware calls return their stable firmware status code",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Firmware,
                    "uefi-runtime-services",
                    firmware == FirmwareMode::Uefi,
                    if firmware == FirmwareMode::Uefi {
                        "uefi-runtime-services"
                    } else {
                        "legacy-bios-services"
                    },
                    if firmware == FirmwareMode::Uefi {
                        "none"
                    } else {
                        "legacy-bios-services"
                    },
                    "time, variables, and reset use the selected firmware contract",
                ),
                DriverCapability::new(
                    DriverCapabilityKind::Timer,
                    "platform-timer-source",
                    true,
                    "shared-monotonic-clock",
                    "shared-monotonic-clock",
                    "APIC, PIT, HPET, pvclock, and scheduling observe one monotonic time source",
                ),
            ],
        }
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
