#![no_std]
#![forbid(unsafe_code)]

pub const DEFAULT_QUEUE_CAPACITY: usize = 64;
pub const MAX_MEDIA_PLANES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    CannotCancel,
    EmptyBuffer,
    InvalidBufferAccess,
    InvalidDevice,
    InvalidFormat,
    InvalidPlaneCount,
    InvalidToken,
    QueueFull,
    RequestNotDispatched,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RequestToken(u64);

impl RequestToken {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Submission<T: Copy> {
    pub token: RequestToken,
    pub request: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Completion<T: Copy> {
    pub token: RequestToken,
    pub result: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotState {
    Vacant,
    Queued,
    Dispatched,
    Completed,
}

#[derive(Clone, Copy)]
struct Slot<Request: Copy, Response: Copy> {
    generation: u32,
    state: SlotState,
    request: Option<Request>,
    result: Option<Response>,
}

impl<Request: Copy, Response: Copy> Slot<Request, Response> {
    const EMPTY: Self = Self {
        generation: 0,
        state: SlotState::Vacant,
        request: None,
        result: None,
    };
}

/// Fixed-capacity asynchronous request/completion queue.
///
/// There is no waiting syscall. Clients submit, services dispatch, and clients
/// poll completions. Generation-checked tokens reject stale completions.
pub struct AsyncQueue<Request: Copy, Response: Copy, const CAPACITY: usize = DEFAULT_QUEUE_CAPACITY>
{
    slots: [Slot<Request, Response>; CAPACITY],
    submit_cursor: usize,
    dispatch_cursor: usize,
    completion_cursor: usize,
}

impl<Request: Copy, Response: Copy, const CAPACITY: usize> AsyncQueue<Request, Response, CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; CAPACITY],
            submit_cursor: 0,
            dispatch_cursor: 0,
            completion_cursor: 0,
        }
    }

    pub fn submit(&mut self, request: Request) -> Result<RequestToken, Error> {
        let slot_index = self
            .find_from(self.submit_cursor, SlotState::Vacant)
            .ok_or(Error::QueueFull)?;
        let slot = &mut self.slots[slot_index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.state = SlotState::Queued;
        slot.request = Some(request);
        slot.result = None;
        self.submit_cursor = Self::next(slot_index);
        Ok(RequestToken::from_parts(slot_index, slot.generation))
    }

    pub fn dispatch(&mut self) -> Option<Submission<Request>> {
        let slot_index = self.find_from(self.dispatch_cursor, SlotState::Queued)?;
        let slot = &mut self.slots[slot_index];
        slot.state = SlotState::Dispatched;
        self.dispatch_cursor = Self::next(slot_index);
        Some(Submission {
            token: RequestToken::from_parts(slot_index, slot.generation),
            request: slot.request.expect("queued request invariant"),
        })
    }

    pub fn complete(&mut self, token: RequestToken, result: Response) -> Result<(), Error> {
        let slot = self.slot_mut(token)?;
        if slot.state != SlotState::Dispatched {
            return Err(Error::RequestNotDispatched);
        }
        slot.state = SlotState::Completed;
        slot.result = Some(result);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Completion<Response>> {
        let slot_index = self.find_from(self.completion_cursor, SlotState::Completed)?;
        let slot = &mut self.slots[slot_index];
        let completion = Completion {
            token: RequestToken::from_parts(slot_index, slot.generation),
            result: slot.result.expect("completed result invariant"),
        };
        slot.state = SlotState::Vacant;
        slot.request = None;
        slot.result = None;
        self.completion_cursor = Self::next(slot_index);
        Some(completion)
    }

    pub fn cancel(&mut self, token: RequestToken) -> Result<(), Error> {
        let slot = self.slot_mut(token)?;
        if slot.state != SlotState::Queued {
            return Err(Error::CannotCancel);
        }
        slot.state = SlotState::Vacant;
        slot.request = None;
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state != SlotState::Vacant)
            .count()
    }

    fn slot_mut(&mut self, token: RequestToken) -> Result<&mut Slot<Request, Response>, Error> {
        let slot = self
            .slots
            .get_mut(token.slot())
            .ok_or(Error::InvalidToken)?;
        if slot.state == SlotState::Vacant || slot.generation != token.generation() {
            return Err(Error::InvalidToken);
        }
        Ok(slot)
    }

    fn find_from(&self, start: usize, state: SlotState) -> Option<usize> {
        if CAPACITY == 0 {
            return None;
        }
        (0..CAPACITY)
            .map(|offset| (start + offset) % CAPACITY)
            .find(|index| self.slots[*index].state == state)
    }

    const fn next(index: usize) -> usize {
        if CAPACITY == 0 {
            0
        } else {
            (index + 1) % CAPACITY
        }
    }
}

impl<Request: Copy, Response: Copy, const CAPACITY: usize> Default
    for AsyncQueue<Request, Response, CAPACITY>
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl BufferAccess {
    pub const fn readable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn writable(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

/// Capability-mapped shared memory. Payload bytes never enter the I/O message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferDescriptor {
    pub region: u32,
    pub offset: u64,
    pub length: u32,
    pub access: BufferAccess,
}

impl BufferDescriptor {
    pub fn validate(self) -> Result<Self, Error> {
        if self.region == 0 || self.length == 0 {
            return Err(Error::EmptyBuffer);
        }
        self.offset
            .checked_add(self.length as u64)
            .ok_or(Error::EmptyBuffer)?;
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoOperation {
    Read,
    Write,
    Flush,
    Control { command: u32, argument: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoRequest {
    pub device: u32,
    pub operation: IoOperation,
    pub device_offset: u64,
    pub buffer: Option<BufferDescriptor>,
}

impl IoRequest {
    pub fn validate(self) -> Result<Self, Error> {
        if self.device == 0 {
            return Err(Error::InvalidDevice);
        }
        match self.operation {
            IoOperation::Read => {
                let buffer = self.buffer.ok_or(Error::EmptyBuffer)?.validate()?;
                if !buffer.access.writable() {
                    return Err(Error::InvalidBufferAccess);
                }
            }
            IoOperation::Write => {
                let buffer = self.buffer.ok_or(Error::EmptyBuffer)?.validate()?;
                if !buffer.access.readable() {
                    return Err(Error::InvalidBufferAccess);
                }
            }
            IoOperation::Control { .. } => {
                if let Some(buffer) = self.buffer {
                    buffer.validate()?;
                }
            }
            IoOperation::Flush => {}
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoResult {
    pub status: u32,
    pub bytes_transferred: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEncoding {
    Signed16,
    Signed24,
    Signed32,
    Float32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoEncoding {
    Rgba8888,
    Bgra8888,
    Nv12,
    Yuv420,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaFormat {
    Audio {
        encoding: AudioEncoding,
        sample_rate: u32,
        channels: u8,
    },
    Video {
        encoding: VideoEncoding,
        width: u32,
        height: u32,
    },
}

impl MediaFormat {
    pub fn validate(self) -> Result<Self, Error> {
        match self {
            Self::Audio {
                sample_rate,
                channels,
                ..
            } if sample_rate == 0 || channels == 0 => Err(Error::InvalidFormat),
            Self::Video { width, height, .. } if width == 0 || height == 0 => {
                Err(Error::InvalidFormat)
            }
            _ => Ok(self),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaOperation {
    Present,
    Capture,
    Encode,
    Decode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaPacket {
    pub stream: u32,
    pub operation: MediaOperation,
    pub format: MediaFormat,
    pub timestamp_ns: u64,
    pub duration_ns: u64,
    pub planes: [Option<BufferDescriptor>; MAX_MEDIA_PLANES],
}

impl MediaPacket {
    pub fn validate(self) -> Result<Self, Error> {
        self.format.validate()?;
        if self.stream == 0 {
            return Err(Error::InvalidDevice);
        }
        let expected_planes = match self.format {
            MediaFormat::Audio { .. } => 1,
            MediaFormat::Video {
                encoding: VideoEncoding::Rgba8888 | VideoEncoding::Bgra8888,
                ..
            } => 1,
            MediaFormat::Video {
                encoding: VideoEncoding::Nv12,
                ..
            } => 2,
            MediaFormat::Video {
                encoding: VideoEncoding::Yuv420,
                ..
            } => 3,
        };
        if self.planes[..expected_planes].iter().any(Option::is_none)
            || self.planes[expected_planes..].iter().any(Option::is_some)
        {
            return Err(Error::InvalidPlaneCount);
        }
        for plane in self.planes.iter().flatten() {
            let plane = plane.validate()?;
            let valid_access = match self.operation {
                MediaOperation::Present | MediaOperation::Encode => plane.access.readable(),
                MediaOperation::Capture | MediaOperation::Decode => plane.access.writable(),
            };
            if !valid_access {
                return Err(Error::InvalidBufferAccess);
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaResult {
    pub status: u32,
    pub timestamp_ns: u64,
    pub processed_bytes: u32,
}

pub struct IoQueue<const CAPACITY: usize = DEFAULT_QUEUE_CAPACITY> {
    queue: AsyncQueue<IoRequest, IoResult, CAPACITY>,
}

impl<const CAPACITY: usize> IoQueue<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            queue: AsyncQueue::new(),
        }
    }

    pub fn submit(&mut self, request: IoRequest) -> Result<RequestToken, Error> {
        self.queue.submit(request.validate()?)
    }

    pub fn dispatch(&mut self) -> Option<Submission<IoRequest>> {
        self.queue.dispatch()
    }

    pub fn complete(&mut self, token: RequestToken, result: IoResult) -> Result<(), Error> {
        self.queue.complete(token, result)
    }

    pub fn poll(&mut self) -> Option<Completion<IoResult>> {
        self.queue.poll()
    }

    pub fn cancel(&mut self, token: RequestToken) -> Result<(), Error> {
        self.queue.cancel(token)
    }

    pub fn pending(&self) -> usize {
        self.queue.pending()
    }
}

impl<const CAPACITY: usize> Default for IoQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MediaQueue<const CAPACITY: usize = DEFAULT_QUEUE_CAPACITY> {
    queue: AsyncQueue<MediaPacket, MediaResult, CAPACITY>,
}

impl<const CAPACITY: usize> MediaQueue<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            queue: AsyncQueue::new(),
        }
    }

    pub fn submit(&mut self, packet: MediaPacket) -> Result<RequestToken, Error> {
        self.queue.submit(packet.validate()?)
    }

    pub fn dispatch(&mut self) -> Option<Submission<MediaPacket>> {
        self.queue.dispatch()
    }

    pub fn complete(&mut self, token: RequestToken, result: MediaResult) -> Result<(), Error> {
        self.queue.complete(token, result)
    }

    pub fn poll(&mut self) -> Option<Completion<MediaResult>> {
        self.queue.poll()
    }

    pub fn cancel(&mut self, token: RequestToken) -> Result<(), Error> {
        self.queue.cancel(token)
    }

    pub fn pending(&self) -> usize {
        self.queue.pending()
    }
}

impl<const CAPACITY: usize> Default for MediaQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
