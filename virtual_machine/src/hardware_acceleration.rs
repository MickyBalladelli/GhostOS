//! Host hardware-acceleration backend selection.
//!
//! Device emulation remains owned by the VM. This module owns the host-side
//! accelerator handle and makes backend selection explicit, so a caller never
//! silently gets a partially configured native backend.

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
}

impl HardwareAccelerationSession {
    /// Negotiate and, for file-backed accelerators, acquire the host handle.
    pub fn open(requested: HardwareAcceleration) -> Result<Self, HardwareAccelerationError> {
        if requested == HardwareAcceleration::Software {
            return Ok(Self {
                requested,
                handle: None,
            });
        }

        if requested == HardwareAcceleration::Auto {
            for &backend in native_backend_order() {
                if let Ok(mut session) = Self::open_native(backend) {
                    session.requested = requested;
                    return Ok(session);
                }
            }
            return Ok(Self {
                requested,
                handle: None,
            });
        }

        Self::open_native(requested)
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
        HardwareAccelerationStatus {
            requested: self.requested,
            active,
            fallback: self.requested == HardwareAcceleration::Auto
                && active == HardwareAcceleration::Software,
            description: if active == HardwareAcceleration::Software {
                "portable software execution"
            } else {
                "native accelerator handle acquired"
            },
        }
    }
}

/// Stable, printable accelerator state for monitors and CLI output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareAccelerationStatus {
    pub requested: HardwareAcceleration,
    pub active: HardwareAcceleration,
    pub fallback: bool,
    pub description: &'static str,
}

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
