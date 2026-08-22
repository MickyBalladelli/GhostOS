use ghostos_platform_io::{BufferAccess, BufferDescriptor};

use crate::DisplayError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PixelFormat {
    Nv12 = 1,
    P010 = 2,
    Rgba8888 = 3,
    Bgra8888 = 4,
}

impl PixelFormat {
    pub const fn plane_count(self) -> usize {
        match self {
            Self::Nv12 | Self::P010 => 2,
            Self::Rgba8888 | Self::Bgra8888 => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramePlane {
    pub buffer: BufferDescriptor,
    pub stride_bytes: u32,
}

impl FramePlane {
    pub fn validate(self) -> Result<Self, DisplayError> {
        self.buffer
            .validate()
            .map_err(|_| DisplayError::InvalidBuffer)?;
        if !self.buffer.access.readable() || self.stride_bytes == 0 {
            return Err(DisplayError::InvalidBuffer);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameDescriptor {
    pub frame_id: u64,
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
    pub timestamp_90khz: u32,
    planes: [Option<FramePlane>; 4],
}

impl FrameDescriptor {
    pub fn new(
        frame_id: u64,
        width: u16,
        height: u16,
        format: PixelFormat,
        timestamp_90khz: u32,
        planes: &[FramePlane],
    ) -> Result<Self, DisplayError> {
        if frame_id == 0 || width == 0 || height == 0 || planes.len() != format.plane_count() {
            return Err(DisplayError::InvalidFrame);
        }
        let mut stored = [None; 4];
        for (index, plane) in planes.iter().enumerate() {
            stored[index] = Some(plane.validate()?)
        }
        let frame = Self {
            frame_id,
            width,
            height,
            format,
            timestamp_90khz,
            planes: stored,
        };
        frame.validate_layout()?;
        Ok(frame)
    }

    pub fn planes(&self) -> impl Iterator<Item = FramePlane> + '_ {
        self.planes.iter().flatten().copied()
    }

    fn validate_layout(&self) -> Result<(), DisplayError> {
        let width = self.width as u32;
        let height = self.height as u32;
        let first = self.planes[0].ok_or(DisplayError::InvalidFrame)?;
        let minimum_stride = match self.format {
            PixelFormat::Nv12 => width,
            PixelFormat::P010 => width.checked_mul(2).ok_or(DisplayError::InvalidFrame)?,
            PixelFormat::Rgba8888 | PixelFormat::Bgra8888 => {
                width.checked_mul(4).ok_or(DisplayError::InvalidFrame)?
            }
        };
        if first.stride_bytes < minimum_stride
            || first.buffer.length < first.stride_bytes.saturating_mul(height)
        {
            return Err(DisplayError::InvalidFrame);
        }
        if let Some(chroma) = self.planes[1] {
            let chroma_rows = height.div_ceil(2);
            if chroma.stride_bytes < minimum_stride
                || chroma.buffer.length < chroma.stride_bytes.saturating_mul(chroma_rows)
            {
                return Err(DisplayError::InvalidFrame);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FrameToken(u64);

impl FrameToken {
    fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrameState {
    Vacant = 0,
    Available = 1,
    Capturing = 2,
    Ready = 3,
    Encoding = 4,
    InFlight = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLease {
    pub token: FrameToken,
    pub descriptor: FrameDescriptor,
}

#[derive(Clone, Copy)]
struct FrameSlot {
    generation: u32,
    state: FrameState,
    descriptor: Option<FrameDescriptor>,
}

impl FrameSlot {
    const EMPTY: Self = Self {
        generation: 0,
        state: FrameState::Vacant,
        descriptor: None,
    };
}

/// Ownership state machine for shared capture surfaces.
pub struct FramePool<const CAPACITY: usize> {
    slots: [FrameSlot; CAPACITY],
}

impl<const CAPACITY: usize> FramePool<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [FrameSlot::EMPTY; CAPACITY],
        }
    }

    pub fn register(&mut self, descriptor: FrameDescriptor) -> Result<FrameToken, DisplayError> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.state == FrameState::Vacant)
            .ok_or(DisplayError::PoolFull)?;
        let slot = &mut self.slots[index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.state = FrameState::Available;
        slot.descriptor = Some(descriptor);
        Ok(FrameToken::from_parts(index, slot.generation))
    }

    pub fn acquire_capture(&mut self) -> Result<FrameLease, DisplayError> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.state == FrameState::Available)
            .ok_or(DisplayError::WouldBlock)?;
        self.transition(index, FrameState::Available, FrameState::Capturing)
    }

    pub fn submit_capture(
        &mut self,
        token: FrameToken,
        frame_id: u64,
        timestamp_90khz: u32,
    ) -> Result<(), DisplayError> {
        let slot = self.slot_mut(token)?;
        if slot.state != FrameState::Capturing || frame_id == 0 {
            return Err(DisplayError::InvalidState);
        }
        let descriptor = slot.descriptor.as_mut().ok_or(DisplayError::InvalidFrame)?;
        descriptor.frame_id = frame_id;
        descriptor.timestamp_90khz = timestamp_90khz;
        slot.state = FrameState::Ready;
        Ok(())
    }

    pub fn acquire_encode(&mut self) -> Result<FrameLease, DisplayError> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.state == FrameState::Ready)
            .ok_or(DisplayError::WouldBlock)?;
        self.transition(index, FrameState::Ready, FrameState::Encoding)
    }

    pub fn mark_in_flight(&mut self, token: FrameToken) -> Result<(), DisplayError> {
        self.set_state(token, FrameState::Encoding, FrameState::InFlight)
    }

    pub fn release(&mut self, token: FrameToken) -> Result<(), DisplayError> {
        self.set_state(token, FrameState::InFlight, FrameState::Available)
    }

    pub fn state(&self, token: FrameToken) -> Result<FrameState, DisplayError> {
        let slot = self
            .slots
            .get(token.slot())
            .ok_or(DisplayError::InvalidFrame)?;
        if slot.state == FrameState::Vacant || slot.generation != token.generation() {
            return Err(DisplayError::InvalidFrame);
        }
        Ok(slot.state)
    }

    pub fn unregister(&mut self, token: FrameToken) -> Result<FrameDescriptor, DisplayError> {
        let slot = self.slot_mut(token)?;
        if slot.state != FrameState::Available {
            return Err(DisplayError::InvalidState);
        }
        slot.state = FrameState::Vacant;
        slot.descriptor.take().ok_or(DisplayError::InvalidFrame)
    }

    fn transition(
        &mut self,
        index: usize,
        expected: FrameState,
        next: FrameState,
    ) -> Result<FrameLease, DisplayError> {
        let slot = &mut self.slots[index];
        if slot.state != expected {
            return Err(DisplayError::InvalidState);
        }
        slot.state = next;
        Ok(FrameLease {
            token: FrameToken::from_parts(index, slot.generation),
            descriptor: slot.descriptor.ok_or(DisplayError::InvalidFrame)?,
        })
    }

    fn set_state(
        &mut self,
        token: FrameToken,
        expected: FrameState,
        next: FrameState,
    ) -> Result<(), DisplayError> {
        let slot = self.slot_mut(token)?;
        if slot.state != expected {
            return Err(DisplayError::InvalidState);
        }
        slot.state = next;
        Ok(())
    }

    fn slot_mut(&mut self, token: FrameToken) -> Result<&mut FrameSlot, DisplayError> {
        let slot = self
            .slots
            .get_mut(token.slot())
            .ok_or(DisplayError::InvalidFrame)?;
        if slot.state == FrameState::Vacant || slot.generation != token.generation() {
            return Err(DisplayError::InvalidFrame);
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for FramePool<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn readable_descriptor(
    buffer: BufferDescriptor,
    bytes: u32,
) -> Result<BufferDescriptor, DisplayError> {
    buffer.validate().map_err(|_| DisplayError::InvalidBuffer)?;
    if buffer.access != BufferAccess::ReadOnly && buffer.access != BufferAccess::ReadWrite
        || bytes == 0
        || bytes > buffer.length
    {
        return Err(DisplayError::InvalidBuffer);
    }
    Ok(buffer)
}
