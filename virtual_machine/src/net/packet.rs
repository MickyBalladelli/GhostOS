//! Packet buffer management.

use std::ffi::CStr;
use std::fmt;
use std::ptr;

#[repr(C)]
struct CPacketQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ghostos_vm_packet_queue_new(max_packets: usize, max_bytes: usize) -> *mut CPacketQueue;
    fn ghostos_vm_packet_queue_free(queue: *mut CPacketQueue);
    fn ghostos_vm_packet_queue_push(queue: *mut CPacketQueue, packet: *const u8, length: usize) -> bool;
    fn ghostos_vm_packet_queue_pop(queue: *mut CPacketQueue) -> bool;
    fn ghostos_vm_packet_queue_peek(queue: *const CPacketQueue, length: *mut usize) -> *const u8;
    fn ghostos_vm_packet_queue_len(queue: *const CPacketQueue) -> usize;
    fn ghostos_vm_packet_queue_bytes(queue: *const CPacketQueue) -> usize;
    fn ghostos_vm_packet_queue_clear(queue: *mut CPacketQueue);
    fn ghostos_vm_packet_pad(
        packet: *const u8,
        length: usize,
        output: *mut u8,
        output_capacity: usize,
        output_length: *mut usize,
    ) -> bool;
    fn ghostos_vm_net_error_message(error: u32) -> *const i8;
}

pub const ETHERNET_FRAME_MAX: usize = 1518;
pub const ETHERNET_FRAME_MIN: usize = 60;
pub const ETHERNET_HEADER_LEN: usize = 14;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    PacketTooLarge,
    Truncated,
    QueueFull,
    LinkDown,
    AdminDown,
    BackendUnavailable,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let message = unsafe { CStr::from_ptr(ghostos_vm_net_error_message(*self as u32)) };
        f.write_str(message.to_str().map_err(|_| fmt::Error)?)
    }
}

impl NetError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::PacketTooLarge => "NET_PACKET_TOO_LARGE",
            Self::Truncated => "NET_TRUNCATED",
            Self::QueueFull => "NET_QUEUE_FULL",
            Self::LinkDown => "NET_LINK_DOWN",
            Self::AdminDown => "NET_ADMIN_DOWN",
            Self::BackendUnavailable => "NET_BACKEND_UNAVAILABLE",
        }
    }
}

impl std::error::Error for NetError {}

pub struct PacketQueue {
    queue: *mut CPacketQueue,
}

// The queue is only accessed through its owning Rust value. Moving ownership
// between threads is safe; sharing a reference concurrently is not exposed.
unsafe impl Send for PacketQueue {}

impl PacketQueue {
    pub fn new(max_packets: usize, max_bytes: usize) -> Self {
        Self { queue: unsafe { ghostos_vm_packet_queue_new(max_packets, max_bytes) } }
    }

    pub fn push(&mut self, packet: Vec<u8>) -> bool {
        !self.queue.is_null() && unsafe {
            ghostos_vm_packet_queue_push(self.queue, packet.as_ptr(), packet.len())
        }
    }

    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let queue = self.queue;
        if queue.is_null() || self.is_empty() {
            return None
        }
        let mut length = 0;
        let data = unsafe { ghostos_vm_packet_queue_peek(queue, &mut length) };
        let data = if data.is_null() { ptr::NonNull::<u8>::dangling().as_ptr() } else { data };
        let packet = unsafe { std::slice::from_raw_parts(data, length).to_vec() };
        unsafe { ghostos_vm_packet_queue_pop(queue) };
        Some(packet)
    }

    pub fn len(&self) -> usize {
        if self.queue.is_null() { 0 } else { unsafe { ghostos_vm_packet_queue_len(self.queue) } }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn peek(&self) -> Option<&[u8]> {
        if self.queue.is_null() || self.is_empty() {
            return None
        }
        let mut length = 0;
        let data = unsafe { ghostos_vm_packet_queue_peek(self.queue, &mut length) };
        let data = if data.is_null() { ptr::NonNull::<u8>::dangling().as_ptr() } else { data };
        Some(unsafe { std::slice::from_raw_parts(data, length) })
    }

    pub fn bytes(&self) -> usize {
        if self.queue.is_null() { 0 } else { unsafe { ghostos_vm_packet_queue_bytes(self.queue) } }
    }

    pub fn clear(&mut self) {
        if !self.queue.is_null() {
            unsafe { ghostos_vm_packet_queue_clear(self.queue) }
        }
    }
}

impl Drop for PacketQueue {
    fn drop(&mut self) {
        if !self.queue.is_null() {
            unsafe { ghostos_vm_packet_queue_free(self.queue) }
        }
    }
}

pub fn pad_frame(packet: &[u8]) -> Vec<u8> {
    let output_length = packet.len().max(ETHERNET_FRAME_MIN);
    let mut output = vec![0; output_length];
    let mut written = 0;
    let ok = unsafe {
        ghostos_vm_packet_pad(
            packet.as_ptr(),
            packet.len(),
            output.as_mut_ptr(),
            output.len(),
            &mut written,
        )
    };
    if ok { output.truncate(written); }
    output
}
