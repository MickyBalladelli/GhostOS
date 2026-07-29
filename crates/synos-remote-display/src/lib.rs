#![no_std]
#![forbid(unsafe_code)]

//! Capability-gated, zero-copy remote display building blocks.
//!
//! Capture surfaces, AV1 encoders, and network drivers exchange shared-memory
//! descriptors. RTP packets carry scatter/gather views into encoded buffers,
//! so frame pixels and encoded payloads never pass through control messages.

mod daemon;
mod encoder;
mod frame;
mod webrtc;

pub use daemon::{
    DisplayDaemon, DisplayGrant, DisplayRights, DisplaySessionId, NetworkFeedback, WebRtcAnswer,
    WebRtcOffer,
};
pub use encoder::{Av1Encoder, Av1Profile, EncodedFrame, EncoderConfig};
pub use frame::{
    FrameDescriptor, FrameLease, FramePlane, FramePool, FrameState, FrameToken, PixelFormat,
};
pub use webrtc::{Av1RtpPacketizer, RtpPacket, RtpPacketError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayError {
    AccessDenied,
    BufferTooSmall,
    Encoder,
    InvalidBuffer,
    InvalidConfiguration,
    InvalidFrame,
    InvalidSession,
    InvalidState,
    NegotiationFailed,
    PoolFull,
    SessionCapacity,
    WouldBlock,
}
