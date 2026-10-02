//! Host hardware-acceleration backend selection.
//!
//! Device emulation remains owned by the VM. This module owns the host-side
//! accelerator handle and makes backend selection explicit. A host handle is
//! not guest execution: the status report says when portable execution is
//! retained, so a caller never silently changes correctness semantics.

use std::fmt;
use std::ffi::{c_void, CStr};
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
        let mut context = ProbeContext { device: None, error: None, attempts: Vec::new() };
        let io = CProbeIo { exists: probe_exists, open: probe_open, api_version: probe_version,
            close: probe_close, attempt: probe_attempt, context: (&mut context as *mut ProbeContext).cast() };
        let host = if cfg!(target_os = "linux") { 1 } else if cfg!(target_os = "macos") { 2 }
            else if cfg!(target_os = "windows") { 3 } else { 0 };
        let mut session = CSession::default();
        if !unsafe { ghostos_vm_acceleration_open(requested as u32, host, &io, &mut session) } {
            return Err(probe_error(&mut context, from_native(session.error_backend), session.error_reason));
        }
        let handle = match from_native(session.active) {
            HardwareAcceleration::Software => None,
            HardwareAcceleration::Kvm => Some(HardwareAccelerationHandle::Kvm {
                device: context.device.take().expect("C selected an opened KVM handle"),
                api_version: session.api_version,
            }),
            HardwareAcceleration::Haxm => Some(HardwareAccelerationHandle::Haxm {
                device: context.device.take().expect("C selected an opened HAXM handle"),
                path: native_path(session.path),
            }),
            HardwareAcceleration::Hvf => Some(HardwareAccelerationHandle::Hvf),
            HardwareAcceleration::Whpx => Some(HardwareAccelerationHandle::Whpx),
            HardwareAcceleration::Auto => unreachable!(),
        };
        Ok(Self { requested, handle, attempts: context.attempts })
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
        let mut status = CStatus::default();
        unsafe { ghostos_vm_acceleration_status_get(self.requested as u32, active as u32, &mut status) };
        let fallback_behavior = match status.fallback {
            0 => HardwareAccelerationFallback::None,
            1 => HardwareAccelerationFallback::NoNativeBackend,
            2 => HardwareAccelerationFallback::NativeExecutionUnavailable,
            _ => unreachable!(),
        };
        HardwareAccelerationStatus {
            requested: self.requested,
            active,
            execution_backend: HardwareAcceleration::Software,
            fallback: !matches!(fallback_behavior, HardwareAccelerationFallback::None),
            fallback_behavior,
            supported_features: if status.feature_count == 5 {
                NATIVE_HANDLE_FEATURES
            } else {
                SOFTWARE_FEATURES
            },
            limitations: if status.limitation_count == 3 {
                NATIVE_HANDLE_LIMITATIONS
            } else {
                SOFTWARE_LIMITATIONS
            },
            attempts: self.attempts.clone(),
            description: unsafe { native_text(ghostos_vm_acceleration_description(status.native_handle)) },
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
        let features: Vec<u32> = self.supported_features.iter().map(|feature| *feature as u32).collect();
        let limitations: Vec<u32> = self.limitations.iter().map(|limitation| *limitation as u32).collect();
        let attempts: Vec<CAttempt> = self.attempts.iter().map(|attempt| CAttempt {
            backend: attempt.backend as u32, available: attempt.available,
            reason: attempt.reason.as_ptr(), reason_length: attempt.reason.len(),
        }).collect();
        let report = CReport {
            requested: self.requested as u32, active: self.active as u32,
            execution: self.execution_backend as u32, fallback: self.fallback_behavior as u32,
            features: features.as_ptr(), feature_count: features.len(),
            limitations: limitations.as_ptr(), limitation_count: limitations.len(),
            attempts: attempts.as_ptr(), attempt_count: attempts.len(),
            description: self.description.as_ptr(), description_length: self.description.len(),
        };
        let mut context = FormatContext { formatter };
        if unsafe { ghostos_vm_acceleration_format(&report, format_write, (&mut context as *mut FormatContext<'_, '_>).cast()) } {
            Ok(())
        } else { Err(fmt::Error) }
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

#[repr(C)]
struct CProbeIo {
    exists: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    open: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    api_version: unsafe extern "C" fn(*mut c_void) -> i32,
    close: unsafe extern "C" fn(*mut c_void),
    attempt: unsafe extern "C" fn(*mut c_void, u32, bool, u32),
    context: *mut c_void,
}
#[repr(C)]
#[derive(Default)]
struct CSession {
    requested: u32,
    active: u32,
    error_backend: u32,
    error_reason: u32,
    api_version: i32,
    path: u32,
}
#[repr(C)]
#[derive(Default)]
struct CStatus {
    fallback: u32,
    feature_count: u32,
    limitation_count: u32,
    native_handle: bool,
}
unsafe extern "C" {
    fn ghostos_vm_acceleration_open(requested: u32, host: u32, io: *const CProbeIo, session: *mut CSession) -> bool;
    fn ghostos_vm_acceleration_status_get(requested: u32, active: u32, status: *mut CStatus);
    fn ghostos_vm_acceleration_reason(reason: u32) -> *const std::ffi::c_char;
    fn ghostos_vm_acceleration_description(native: bool) -> *const std::ffi::c_char;
    fn ghostos_vm_acceleration_path(path: u32) -> *const std::ffi::c_char;
    #[cfg(target_os = "linux")]
    fn ghostos_vm_acceleration_kvm_version(fd: i32) -> i32;
}

// These pointers refer only to process-lifetime C string literals.
unsafe fn native_text(pointer: *const std::ffi::c_char) -> &'static str {
    unsafe { CStr::from_ptr(pointer) }.to_str().expect("C accelerator text is UTF-8")
}
fn native_path(path: u32) -> &'static str { unsafe { native_text(ghostos_vm_acceleration_path(path)) } }
fn from_native(value: u32) -> HardwareAcceleration {
    match value {
        0 => HardwareAcceleration::Software,
        1 => HardwareAcceleration::Auto,
        2 => HardwareAcceleration::Kvm,
        3 => HardwareAcceleration::Haxm,
        4 => HardwareAcceleration::Hvf,
        5 => HardwareAcceleration::Whpx,
        _ => unreachable!(),
    }
}
struct ProbeContext {
    device: Option<File>,
    error: Option<std::io::Error>,
    attempts: Vec<HardwareAccelerationAttempt>,
}
fn probe_error(context: &mut ProbeContext, backend: HardwareAcceleration, reason: u32) -> HardwareAccelerationError {
    if reason == 6 {
        HardwareAccelerationError::Io { backend, source: context.error.take().expect("failed C probe retained its I/O error") }
    } else {
        HardwareAccelerationError::Unavailable { backend,
            reason: unsafe { native_text(ghostos_vm_acceleration_reason(reason)) }.to_string() }
    }
}
unsafe extern "C" fn probe_exists(_raw: *mut c_void, path: u32) -> bool {
    Path::new(native_path(path)).exists()
}
unsafe extern "C" fn probe_open(raw: *mut c_void, path: u32) -> bool {
    let context = unsafe { &mut *raw.cast::<ProbeContext>() };
    match File::options().read(true).write(true).open(native_path(path)) {
        Ok(device) => { context.device = Some(device); true }
        Err(error) => { context.error = Some(error); false }
    }
}
unsafe extern "C" fn probe_version(raw: *mut c_void) -> i32 {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let context = unsafe { &mut *raw.cast::<ProbeContext>() };
        unsafe { ghostos_vm_acceleration_kvm_version(context.device.as_ref().expect("opened KVM handle").as_raw_fd()) }
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = raw; -1 }
}
unsafe extern "C" fn probe_close(raw: *mut c_void) {
    let context = unsafe { &mut *raw.cast::<ProbeContext>() };
    context.device = None;
}
unsafe extern "C" fn probe_attempt(raw: *mut c_void, backend: u32, available: bool, reason: u32) {
    let context = unsafe { &mut *raw.cast::<ProbeContext>() };
    let backend = from_native(backend);
    let reason = if available { unsafe { native_text(ghostos_vm_acceleration_reason(0)) }.to_string() }
        else { probe_error(context, backend, reason).to_string() };
    context.attempts.push(HardwareAccelerationAttempt { backend, available, reason });
}

#[repr(C)]
struct CAttempt { backend: u32, available: bool, reason: *const u8, reason_length: usize }
#[repr(C)]
struct CReport {
    requested: u32, active: u32, execution: u32, fallback: u32,
    features: *const u32, feature_count: usize,
    limitations: *const u32, limitation_count: usize,
    attempts: *const CAttempt, attempt_count: usize,
    description: *const u8, description_length: usize,
}
unsafe extern "C" {
    fn ghostos_vm_acceleration_format(report: *const CReport,
        write: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> bool, context: *mut c_void) -> bool;
}
struct FormatContext<'a, 'b> { formatter: &'a mut fmt::Formatter<'b> }
unsafe extern "C" fn format_write(raw: *mut c_void, bytes: *const u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<FormatContext<'_, '_>>() };
    let bytes = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } };
    match std::str::from_utf8(bytes) {
        Ok(text) => context.formatter.write_str(text).is_ok(),
        Err(_) => false,
    }
}
