//! Host hardware-acceleration backend selection.
//!
//! Device emulation remains owned by the VM. This module owns the host-side
//! accelerator handle and makes backend selection explicit. A host handle is
//! not guest execution: the status report says when portable execution is
//! retained, so a caller never silently changes correctness semantics.

use std::fmt;
use std::fs::File;
use std::path::Path;

/// Host execution backend requested for a VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HardwareAcceleration {
    /// Keep using the portable Rust CPU executor.
    #[default]
    Software,
    /// Select the first native backend supported by this host.
    Auto,
    /// Linux Kernel-based Virtual Machine.
    Kvm,
    /// Intel Hardware Accelerated Execution Manager.
    Haxm,
    /// Apple Hypervisor Framework.
    Hvf,
    /// Windows Hypervisor Platform.
    Whpx,
}

impl HardwareAcceleration {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Auto => "auto",
            Self::Kvm => "kvm",
            Self::Haxm => "haxm",
            Self::Hvf => "hvf",
            Self::Whpx => "whpx",
        }
    }

    pub const fn is_native(self) -> bool {
        !matches!(self, Self::Software)
    }
}

impl fmt::Display for HardwareAcceleration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A native host handle acquired for the selected backend.
#[derive(Debug)]
pub enum HardwareAccelerationHandle {
    /// `/dev/kvm` opened and validated with KVM_GET_API_VERSION.
    Kvm { device: File, api_version: i32 },
    /// HAXM device node opened for the VM process.
    Haxm { device: File, path: &'static str },
    /// Hypervisor.framework is a system framework on supported macOS hosts.
    Hvf,
    /// Windows Hypervisor Platform is an OS facility on supported Windows hosts.
    Whpx,
}

impl HardwareAccelerationHandle {
    pub const fn backend(&self) -> HardwareAcceleration {
        match self {
            Self::Kvm { .. } => HardwareAcceleration::Kvm,
            Self::Haxm { .. } => HardwareAcceleration::Haxm,
            Self::Hvf => HardwareAcceleration::Hvf,
            Self::Whpx => HardwareAcceleration::Whpx,
        }
    }
}

/// Result of backend negotiation and host capability probing.
#[derive(Debug)]
pub struct HardwareAccelerationSession {
    requested: HardwareAcceleration,
    handle: Option<HardwareAccelerationHandle>,
    attempts: Vec<HardwareAccelerationAttempt>,
}

impl HardwareAccelerationSession {
    /// Negotiate and, for file-backed accelerators, acquire the host handle.
    /// The handle is not connected to guest execution; inspect [`Self::status`]
    /// for the actual execution backend and fallback.
    pub fn open(requested: HardwareAcceleration) -> Result<Self, HardwareAccelerationError> {
        if requested == HardwareAcceleration::Software {
            return Ok(Self {
                requested,
                handle: None,
                attempts: Vec::new(),
            });
        }

        if requested == HardwareAcceleration::Auto {
            let mut attempts = Vec::new();
            for &backend in native_backend_order() {
                match Self::open_native(backend) {
                    Ok(mut session) => {
                        attempts.push(HardwareAccelerationAttempt::available(backend));
                        session.requested = requested;
                        session.attempts = attempts;
                        return Ok(session);
                    }
                    Err(error) => {
                        attempts.push(HardwareAccelerationAttempt::unavailable(
                            backend,
                            error.to_string(),
                        ));
                    }
                }
            }
            return Ok(Self {
                requested,
                handle: None,
                attempts,
            });
        }

        let mut session = Self::open_native(requested)?;
        session.attempts.push(HardwareAccelerationAttempt::available(requested));
        Ok(session)
    }

    fn open_native(backend: HardwareAcceleration) -> Result<Self, HardwareAccelerationError> {
        let handle = match backend {
            HardwareAcceleration::Software | HardwareAcceleration::Auto => unreachable!(),
            HardwareAcceleration::Kvm => open_kvm()?,
            HardwareAcceleration::Haxm => open_haxm()?,
            HardwareAcceleration::Hvf => {
                if cfg!(target_os = "macos") {
                    HardwareAccelerationHandle::Hvf
                } else {
                    return Err(unsupported(backend, "HVF requires macOS"));
                }
            }
            HardwareAcceleration::Whpx => {
                if cfg!(target_os = "windows") {
                    HardwareAccelerationHandle::Whpx
                } else {
                    return Err(unsupported(backend, "WHPX requires Windows"));
                }
            }
        };
        Ok(Self {
            requested: backend,
            handle: Some(handle),
            attempts: Vec::new(),
        })
    }

    pub const fn requested(&self) -> HardwareAcceleration {
        self.requested
    }

    pub fn active_backend(&self) -> HardwareAcceleration {
        self.handle
            .as_ref()
            .map(HardwareAccelerationHandle::backend)
            .unwrap_or(HardwareAcceleration::Software)
    }

    pub const fn is_active(&self) -> bool {
        self.handle.is_some()
    }

    pub fn handle(&self) -> Option<&HardwareAccelerationHandle> {
        self.handle.as_ref()
    }

    pub fn status(&self) -> HardwareAccelerationStatus {
        let active = self.active_backend();
        let native_handle = active != HardwareAcceleration::Software;
        let fallback_behavior = match (self.requested, native_handle) {
            (HardwareAcceleration::Software, _) => HardwareAccelerationFallback::None,
            (_, false) => HardwareAccelerationFallback::NoNativeBackend,
            (_, true) => HardwareAccelerationFallback::NativeExecutionUnavailable,
        };
        HardwareAccelerationStatus {
            requested: self.requested,
            active,
            execution_backend: HardwareAcceleration::Software,
            fallback: !matches!(fallback_behavior, HardwareAccelerationFallback::None),
            fallback_behavior,
            supported_features: if native_handle {
                NATIVE_HANDLE_FEATURES
            } else {
                SOFTWARE_FEATURES
            },
            limitations: if native_handle {
                NATIVE_HANDLE_LIMITATIONS
            } else {
                SOFTWARE_LIMITATIONS
            },
            attempts: self.attempts.clone(),
            description: if native_handle {
                "native host handle acquired; portable software execution remains active"
            } else {
                "portable software execution"
            },
        }
    }
}

/// A capability that the VM can honestly provide for the selected mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareAccelerationFeature {
    /// Guest instructions run through the portable CPU executor.
    PortableCpuExecution,
    /// Guest devices remain implemented by the VM's device models.
    GuestDeviceEmulation,
    /// Replay can keep using deterministic VM-owned inputs and events.
    DeterministicReplay,
    /// Snapshots can restore VM state and rebuild host-side handles.
    SnapshotRestore,
    /// A host accelerator interface was opened and retained by the session.
    NativeHostHandle,
}

impl HardwareAccelerationFeature {
    pub const fn name(self) -> &'static str {
        match self {
            Self::PortableCpuExecution => "portable-cpu-execution",
            Self::GuestDeviceEmulation => "guest-device-emulation",
            Self::DeterministicReplay => "deterministic-replay",
            Self::SnapshotRestore => "snapshot-restore",
            Self::NativeHostHandle => "native-host-handle",
        }
    }
}

impl fmt::Display for HardwareAccelerationFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A limitation that affects what a selected host accelerator changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareAccelerationLimitation {
    /// The host handle is not connected to a native guest CPU executor yet.
    NativeExecutionNotIntegrated,
    /// Guest memory and devices are not backed by host virtualization yet.
    NativeMemoryAndDeviceVirtualizationNotIntegrated,
    /// Native handles are host state and are reopened after snapshot restore.
    NativeHandleReopenedOnSnapshotRestore,
}

impl HardwareAccelerationLimitation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::NativeExecutionNotIntegrated => "native-execution-not-integrated",
            Self::NativeMemoryAndDeviceVirtualizationNotIntegrated => {
                "native-memory-and-device-virtualization-not-integrated"
            }
            Self::NativeHandleReopenedOnSnapshotRestore => {
                "native-handle-reopened-on-snapshot-restore"
            }
        }
    }
}

impl fmt::Display for HardwareAccelerationLimitation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The explicit fallback taken by a hardware-acceleration request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareAccelerationFallback {
    /// Software was requested, so no fallback was needed.
    None,
    /// `auto` found no usable native host interface.
    NoNativeBackend,
    /// A host handle exists, but native VM execution is not integrated.
    NativeExecutionUnavailable,
}

impl HardwareAccelerationFallback {
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NoNativeBackend => "no-native-backend",
            Self::NativeExecutionUnavailable => "native-execution-unavailable",
        }
    }
}

impl fmt::Display for HardwareAccelerationFallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One native backend considered during host capability negotiation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardwareAccelerationAttempt {
    pub backend: HardwareAcceleration,
    pub available: bool,
    pub reason: String,
}

impl HardwareAccelerationAttempt {
    fn available(backend: HardwareAcceleration) -> Self {
        Self {
            backend,
            available: true,
            reason: "host accelerator handle acquired".to_string(),
        }
    }

    fn unavailable(backend: HardwareAcceleration, reason: String) -> Self {
        Self {
            backend,
            available: false,
            reason,
        }
    }
}

/// Stable, printable accelerator state for monitors and CLI output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardwareAccelerationStatus {
    pub requested: HardwareAcceleration,
    /// Host backend whose interface was selected, or software when none was.
    pub active: HardwareAcceleration,
    /// Backend actually executing guest instructions. This is software until
    /// a native CPU executor is integrated and equivalence-tested.
    pub execution_backend: HardwareAcceleration,
    /// Kept for callers that only need a simple fallback flag.
    pub fallback: bool,
    pub fallback_behavior: HardwareAccelerationFallback,
    pub supported_features: &'static [HardwareAccelerationFeature],
    pub limitations: &'static [HardwareAccelerationLimitation],
    pub attempts: Vec<HardwareAccelerationAttempt>,
    pub description: &'static str,
}

impl fmt::Display for HardwareAccelerationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "requested={} host={} execution={} fallback={} supported=",
            self.requested, self.active, self.execution_backend, self.fallback_behavior,
        )?;
        for (index, feature) in self.supported_features.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{feature}")?;
        }
        formatter.write_str(" limitations=")?;
        for (index, limitation) in self.limitations.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{limitation}")?;
        }
        if !self.attempts.is_empty() {
            formatter.write_str(" attempts=")?;
            for (index, attempt) in self.attempts.iter().enumerate() {
                if index > 0 {
                    formatter.write_str(",")?;
                }
                write!(
                    formatter,
                    "{}:{}({})",
                    attempt.backend,
                    if attempt.available { "available" } else { "unavailable" },
                    attempt.reason,
                )?;
            }
        }
        write!(formatter, " ({})", self.description)
    }
}

const SOFTWARE_FEATURES: &[HardwareAccelerationFeature] = &[
    HardwareAccelerationFeature::PortableCpuExecution,
    HardwareAccelerationFeature::GuestDeviceEmulation,
    HardwareAccelerationFeature::DeterministicReplay,
    HardwareAccelerationFeature::SnapshotRestore,
];

const NATIVE_HANDLE_FEATURES: &[HardwareAccelerationFeature] = &[
    HardwareAccelerationFeature::PortableCpuExecution,
    HardwareAccelerationFeature::GuestDeviceEmulation,
    HardwareAccelerationFeature::DeterministicReplay,
    HardwareAccelerationFeature::SnapshotRestore,
    HardwareAccelerationFeature::NativeHostHandle,
];

const SOFTWARE_LIMITATIONS: &[HardwareAccelerationLimitation] = &[
    HardwareAccelerationLimitation::NativeExecutionNotIntegrated,
];

const NATIVE_HANDLE_LIMITATIONS: &[HardwareAccelerationLimitation] = &[
    HardwareAccelerationLimitation::NativeExecutionNotIntegrated,
    HardwareAccelerationLimitation::NativeMemoryAndDeviceVirtualizationNotIntegrated,
    HardwareAccelerationLimitation::NativeHandleReopenedOnSnapshotRestore,
];

#[derive(Debug, thiserror::Error)]
pub enum HardwareAccelerationError {
    #[error("{backend} is unavailable: {reason}")]
    Unavailable {
        backend: HardwareAcceleration,
        reason: String,
    },
    #[error("cannot open {backend}: {source}")]
    Io {
        backend: HardwareAcceleration,
        #[source]
        source: std::io::Error,
    },
}

fn unsupported(backend: HardwareAcceleration, reason: &str) -> HardwareAccelerationError {
    HardwareAccelerationError::Unavailable {
        backend,
        reason: reason.to_string(),
    }
}

fn native_backend_order() -> &'static [HardwareAcceleration] {
    #[cfg(target_os = "linux")]
    {
        &[HardwareAcceleration::Kvm, HardwareAcceleration::Haxm]
    }
    #[cfg(target_os = "macos")]
    {
        &[HardwareAcceleration::Hvf, HardwareAcceleration::Haxm]
    }
    #[cfg(target_os = "windows")]
    {
        &[HardwareAcceleration::Whpx, HardwareAcceleration::Haxm]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        &[]
    }
}

fn open_kvm() -> Result<HardwareAccelerationHandle, HardwareAccelerationError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;

        let device = File::options()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .map_err(|source| HardwareAccelerationError::Io {
                backend: HardwareAcceleration::Kvm,
                source,
            })?;
        let api_version = unsafe { kvm_get_api_version(device.as_raw_fd()) };
        if api_version < 12 {
            return Err(unsupported(
                HardwareAcceleration::Kvm,
                "KVM API version is too old",
            ));
        }
        return Ok(HardwareAccelerationHandle::Kvm {
            device,
            api_version,
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(unsupported(
            HardwareAcceleration::Kvm,
            "KVM requires Linux and /dev/kvm",
        ))
    }
}

fn open_haxm() -> Result<HardwareAccelerationHandle, HardwareAccelerationError> {
    for path in ["/dev/HAXM", "/dev/haxm"] {
        if Path::new(path).exists() {
            if let Ok(device) = File::options().read(true).write(true).open(path) {
                return Ok(HardwareAccelerationHandle::Haxm { device, path });
            }
        }
    }
    Err(unsupported(
        HardwareAcceleration::Haxm,
        "HAXM device node was not found",
    ))
}

#[cfg(target_os = "linux")]
const KVM_GET_API_VERSION: std::os::raw::c_ulong = 0xAE00;

#[cfg(target_os = "linux")]
unsafe fn kvm_get_api_version(fd: std::os::raw::c_int) -> std::os::raw::c_int {
    extern "C" {
        fn ioctl(
            fd: std::os::raw::c_int,
            request: std::os::raw::c_ulong,
            ...
        ) -> std::os::raw::c_int;
    }
    unsafe { ioctl(fd, KVM_GET_API_VERSION) }
}
