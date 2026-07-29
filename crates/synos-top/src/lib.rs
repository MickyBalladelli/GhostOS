#![no_std]
#![forbid(unsafe_code)]

//! Heap-free cluster topology dashboard.
//!
//! `synos-top` separates live sampling from presentation. A user-space daemon
//! can update [`TopologySnapshot`] from capability-protected services, then
//! render the same ANSI stream to a serial terminal, VGA console, or GOP
//! framebuffer console.

mod model;
mod render;
mod runtime;
mod telemetry;

pub use model::{
    CapabilityKind, CapabilitySample, NodeHealth, NodeSample, SnapshotError,
    TopologySnapshot, DEFAULT_CAPABILITY_CAPACITY, DEFAULT_NODE_CAPACITY,
};
pub use render::{DashboardRenderer, RenderError, Viewport};
pub use runtime::{Poll, RealtimeMonitor, TopologySource};
pub use telemetry::{DsmLatencySample, DsmLatencyTracker};
