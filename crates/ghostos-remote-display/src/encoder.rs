use ghostos_platform_io::BufferDescriptor;

use crate::{DisplayError, FrameDescriptor, frame::readable_descriptor};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Av1Profile {
    Main8 = 0,
    Main10 = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncoderConfig {
    pub profile: Av1Profile,
    pub width: u16,
    pub height: u16,
    pub frames_per_second: u8,
    pub bitrate_bits_per_second: u32,
    pub keyframe_interval: u16,
    pub low_latency: bool,
}

impl EncoderConfig {
    pub const fn validate(self) -> Result<Self, DisplayError> {
        if self.width == 0
            || self.height == 0
            || self.frames_per_second == 0
            || self.frames_per_second > 120
            || self.bitrate_bits_per_second < 64_000
            || self.keyframe_interval == 0
        {
            Err(DisplayError::InvalidConfiguration)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodedFrame {
    pub frame_id: u64,
    pub timestamp_90khz: u32,
    pub buffer: BufferDescriptor,
    pub bytes: u32,
    pub keyframe: bool,
    pub temporal_id: u8,
}

impl EncodedFrame {
    pub fn validate(self) -> Result<Self, DisplayError> {
        if self.frame_id == 0 || self.temporal_id > 3 {
            return Err(DisplayError::InvalidFrame);
        }
        readable_descriptor(self.buffer, self.bytes)?;
        Ok(self)
    }
}

/// Hardware or software AV1 encoder boundary.
///
/// Implementations receive shared capture planes and return a shared encoded
/// bitstream descriptor. They must not copy frame pixels through IPC.
pub trait Av1Encoder {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), DisplayError>;
    fn encode(
        &mut self,
        frame: FrameDescriptor,
        force_keyframe: bool,
    ) -> Result<EncodedFrame, DisplayError>;
    fn drain(&mut self) -> Result<(), DisplayError>;
}
