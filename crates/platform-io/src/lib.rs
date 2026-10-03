#![no_std]

use ghostos_status::{IntoStatus, Severity, Status, facility};
pub use ghostos_numa::{
    NumaCounters, NumaDecision, NumaPlacement, NumaReport, NumaTopology, NumaTopologyError,
    PlacementKind, PlacementLocality,
};
use ghostos_observability::{field, EventField, EventKind};

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

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::QueueFull => Status::BUSY,
            Self::InvalidDevice => Status::NOT_FOUND,
            Self::CannotCancel | Self::RequestNotDispatched => {
                Status::new(Severity::Warning, facility::DRIVER, 1, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::EmptyBuffer
            | Self::InvalidBufferAccess
            | Self::InvalidFormat
            | Self::InvalidPlaneCount
            | Self::InvalidToken => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RequestToken(u64);

impl RequestToken {
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

#[repr(C)]
#[derive(Clone, Copy)]
struct SlotMetadata {
    generation: u32,
    state: u32,
}

impl SlotMetadata {
    const EMPTY: Self = Self { generation: 0, state: 0 };
}

#[repr(C)]
struct QueueCursors {
    submit: usize,
    dispatch: usize,
    completion: usize,
}

#[derive(Clone, Copy)]
struct Slot<Request: Copy, Response: Copy> {
    request: Option<Request>,
    result: Option<Response>,
}

impl<Request: Copy, Response: Copy> Slot<Request, Response> {
    const EMPTY: Self = Self { request: None, result: None };
}

/// Fixed-capacity asynchronous request/completion queue.
///
/// There is no waiting syscall. Clients submit, services dispatch, and clients
/// poll completions. Generation-checked tokens reject stale completions.
pub struct AsyncQueue<Request: Copy, Response: Copy, const CAPACITY: usize = DEFAULT_QUEUE_CAPACITY>
{
    slots: [Slot<Request, Response>; CAPACITY],
    metadata: [SlotMetadata; CAPACITY],
    cursors: QueueCursors,
    numa: NumaPlacement,
    placement: NumaDecision,
}

impl<Request: Copy, Response: Copy, const CAPACITY: usize> AsyncQueue<Request, Response, CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; CAPACITY],
            metadata: [SlotMetadata::EMPTY; CAPACITY],
            cursors: QueueCursors { submit: 0, dispatch: 0, completion: 0 },
            numa: NumaPlacement::uma(),
            placement: NumaDecision::uma(PlacementKind::Queue),
        }
    }

    /// Bind queue ownership to the closest CPU/node and retain the decision
    /// for inspection. The UMA result is explicit when topology is absent.
    pub fn configure_numa(
        &mut self,
        topology: NumaTopology,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        self.numa.set_topology(topology);
        self.placement = self
            .numa
            .place(PlacementKind::Queue, preferred_cpu, preferred_node);
        ghostos_observability::info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::Queue as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, self.placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, self.placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, self.placement.locality as u64),
        );
        self.placement
    }

    pub const fn placement(&self) -> NumaDecision {
        self.placement
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.numa.report()
    }

    pub fn submit(&mut self, request: Request) -> Result<RequestToken, Error> {
        let mut index = 0;
        let mut token = 0;
        // C borrows only checked metadata records; payloads remain Rust-owned.
        io_result(unsafe {
            ghostos_io_submit(self.metadata.as_mut_ptr(), CAPACITY,
                &mut self.cursors, &mut index, &mut token)
        })?;
        self.slots[index].request = Some(request);
        self.slots[index].result = None;
        Ok(RequestToken(token))
    }

    pub fn dispatch(&mut self) -> Option<Submission<Request>> {
        let mut index = 0;
        let mut token = 0;
        // Queued slots always have a request published by submit.
        if !unsafe {
            ghostos_io_dispatch(self.metadata.as_mut_ptr(), CAPACITY,
                &mut self.cursors, &mut index, &mut token)
        } {
            return None
        }
        Some(Submission { token: RequestToken(token), request: self.slots[index].request? })
    }

    pub fn complete(&mut self, token: RequestToken, result: Response) -> Result<(), Error> {
        // C validates the token and dispatched state before the payload is written.
        io_result(unsafe {
            ghostos_io_complete(self.metadata.as_mut_ptr(), CAPACITY, token.raw())
        })?;
        self.slots[token.raw() as u32 as usize].result = Some(result);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Completion<Response>> {
        let mut index = 0;
        let mut token = 0;
        // C releases only a completed slot and advances the completion cursor.
        if !unsafe {
            ghostos_io_poll(self.metadata.as_mut_ptr(), CAPACITY,
                &mut self.cursors, &mut index, &mut token)
        } {
            return None
        }
        let slot = &mut self.slots[index];
        slot.request = None;
        let result = slot.result.take()?;
        Some(Completion { token: RequestToken(token), result })
    }

    pub fn cancel(&mut self, token: RequestToken) -> Result<(), Error> {
        // C rejects stale tokens and every state except queued before mutation.
        io_result(unsafe {
            ghostos_io_cancel(self.metadata.as_mut_ptr(), CAPACITY, token.raw())
        })?;
        self.slots[token.raw() as u32 as usize].request = None;
        Ok(())
    }

    pub fn pending(&self) -> usize {
        // C reads metadata only and retains no pointer.
        unsafe { ghostos_io_pending(self.metadata.as_ptr(), CAPACITY) }
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
#[repr(u8)]
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
#[repr(C)]
pub struct BufferDescriptor {
    pub region: u32,
    pub offset: u64,
    pub length: u32,
    pub access: BufferAccess,
}

impl BufferDescriptor {
    pub fn validate(self) -> Result<Self, Error> {
        // C reads this checked C-layout descriptor without retaining it.
        io_result(unsafe { ghostos_io_buffer_validate(&self) })?;
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
        let operation = match self.operation {
            IoOperation::Read => 0,
            IoOperation::Write => 1,
            IoOperation::Flush => 2,
            IoOperation::Control { .. } => 3,
        };
        let buffer = self.buffer.as_ref().map_or(core::ptr::null(), |buffer| buffer as *const _);
        // The optional descriptor lives through this call; C retains no pointers.
        io_result(unsafe { ghostos_io_request_validate(self.device, operation, buffer) })?;
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
        let (video, _, first, second) = self.c_parts();
        io_result(unsafe { ghostos_media_format_validate(video, first, second) })?;
        Ok(self)
    }

    fn c_parts(self) -> (bool, u8, u32, u32) {
        match self {
            Self::Audio { sample_rate, channels, .. } => (false, 0, sample_rate, channels as u32),
            Self::Video { encoding, width, height } => {
                let encoding = match encoding {
                    VideoEncoding::Rgba8888 => 0,
                    VideoEncoding::Bgra8888 => 1,
                    VideoEncoding::Nv12 => 2,
                    VideoEncoding::Yuv420 => 3,
                };
                (true, encoding, width, height)
            }
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
        let (video, encoding, first, second) = self.format.c_parts();
        let operation = match self.operation {
            MediaOperation::Present => 0,
            MediaOperation::Capture => 1,
            MediaOperation::Encode => 2,
            MediaOperation::Decode => 3,
        };
        let empty = BufferDescriptor { region: 0, offset: 0, length: 0, access: BufferAccess::ReadOnly };
        let planes = self.planes.map(|plane| plane.unwrap_or(empty));
        let present = self.planes.map(|plane| plane.is_some());
        // C reads four descriptors and presence flags in index order.
        io_result(unsafe {
            ghostos_media_packet_validate(self.stream, operation, video, encoding,
                first, second, planes.as_ptr(), present.as_ptr())
        })?;
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

    pub fn configure_numa(
        &mut self,
        topology: NumaTopology,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        self.queue.configure_numa(topology, preferred_cpu, preferred_node)
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.queue.numa_report()
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

    pub fn configure_numa(
        &mut self,
        topology: NumaTopology,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        self.queue.configure_numa(topology, preferred_cpu, preferred_node)
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.queue.numa_report()
    }
}

impl<const CAPACITY: usize> Default for MediaQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn io_result(code: u32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        1 => Err(Error::CannotCancel),
        2 => Err(Error::EmptyBuffer),
        3 => Err(Error::InvalidBufferAccess),
        4 => Err(Error::InvalidDevice),
        5 => Err(Error::InvalidFormat),
        6 => Err(Error::InvalidPlaneCount),
        7 => Err(Error::InvalidToken),
        8 => Err(Error::QueueFull),
        _ => Err(Error::RequestNotDispatched),
    }
}

const _: () = {
    assert!(core::mem::size_of::<SlotMetadata>() == 8);
    assert!(core::mem::offset_of!(SlotMetadata, state) == 4);
    assert!(core::mem::size_of::<QueueCursors>() == 3 * core::mem::size_of::<usize>());
    assert!(core::mem::offset_of!(QueueCursors, dispatch) == core::mem::size_of::<usize>());
    assert!(core::mem::offset_of!(QueueCursors, completion) == 2 * core::mem::size_of::<usize>());
    assert!(core::mem::size_of::<BufferDescriptor>() == 24);
    assert!(core::mem::offset_of!(BufferDescriptor, offset) == 8);
    assert!(core::mem::offset_of!(BufferDescriptor, length) == 16);
    assert!(core::mem::offset_of!(BufferDescriptor, access) == 20);
};

unsafe extern "C" {
    fn ghostos_io_submit(slots: *mut SlotMetadata, capacity: usize,
        cursors: *mut QueueCursors, index: *mut usize, token: *mut u64) -> u32;
    fn ghostos_io_dispatch(slots: *mut SlotMetadata, capacity: usize,
        cursors: *mut QueueCursors, index: *mut usize, token: *mut u64) -> bool;
    fn ghostos_io_complete(slots: *mut SlotMetadata, capacity: usize, token: u64) -> u32;
    fn ghostos_io_poll(slots: *mut SlotMetadata, capacity: usize,
        cursors: *mut QueueCursors, index: *mut usize, token: *mut u64) -> bool;
    fn ghostos_io_cancel(slots: *mut SlotMetadata, capacity: usize, token: u64) -> u32;
    fn ghostos_io_pending(slots: *const SlotMetadata, capacity: usize) -> usize;
    fn ghostos_io_buffer_validate(buffer: *const BufferDescriptor) -> u32;
    fn ghostos_io_request_validate(device: u32, operation: u8, buffer: *const BufferDescriptor) -> u32;
    fn ghostos_media_format_validate(video: bool, first: u32, second: u32) -> u32;
    fn ghostos_media_packet_validate(stream: u32, operation: u8, video: bool,
        encoding: u8, first: u32, second: u32, planes: *const BufferDescriptor,
        present: *const bool) -> u32;
}
