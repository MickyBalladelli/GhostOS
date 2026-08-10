//! Bounded packet and control-plane evidence for deterministic network tests.

use crate::dhcp::MAX_DHCP_PACKET;

pub const MAX_CAPTURE_RECORDS: usize = 32;
pub const MAX_CAPTURE_BYTES: usize = MAX_DHCP_PACKET;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureDirection {
    Ingress,
    Egress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CaptureKind {
    Packet = 0,
    NetworkCommand = 1,
    DhcpLifecycle = 2,
    FirewallDecision = 3,
    Rollback = 4,
    Link = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DhcpLifecycleEvent {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Renew = 8,
    Rebind = 9,
    Rollback = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureRecord {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub kind: CaptureKind,
    pub direction: Option<CaptureDirection>,
    pub source_ip: [u8; 4],
    pub destination_ip: [u8; 4],
    pub source_mac: [u8; 6],
    pub destination_mac: [u8; 6],
    pub source_port: u16,
    pub destination_port: u16,
    pub code: u16,
    pub length: usize,
    pub original_length: usize,
    pub truncated: bool,
    pub payload: [u8; MAX_CAPTURE_BYTES],
}

impl CaptureRecord {
    const EMPTY: Self = Self {
        sequence: 0,
        timestamp_ms: 0,
        kind: CaptureKind::Packet,
        direction: None,
        source_ip: [0; 4],
        destination_ip: [0; 4],
        source_mac: [0; 6],
        destination_mac: [0; 6],
        source_port: 0,
        destination_port: 0,
        code: 0,
        length: 0,
        original_length: 0,
        truncated: false,
        payload: [0; MAX_CAPTURE_BYTES],
    };

    pub fn bytes(&self) -> &[u8] {
        &self.payload[..self.length]
    }
}

/// Fixed-size evidence buffer. New records are appended in order until full;
/// dropped-record count makes truncation visible instead of silently losing it.
pub struct PacketCapture<const CAPACITY: usize = MAX_CAPTURE_RECORDS> {
    records: [CaptureRecord; CAPACITY],
    length: usize,
    next_sequence: u64,
    dropped: u64,
}

impl<const CAPACITY: usize> PacketCapture<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [CaptureRecord::EMPTY; CAPACITY],
            length: 0,
            next_sequence: 0,
            dropped: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn records(&self) -> &[CaptureRecord] {
        &self.records[..self.length]
    }

    pub fn record_packet(
        &mut self,
        timestamp_ms: u64,
        direction: CaptureDirection,
        source_mac: [u8; 6],
        destination_mac: [u8; 6],
        source_ip: [u8; 4],
        destination_ip: [u8; 4],
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) {
        self.record(CaptureRecord {
            timestamp_ms,
            kind: CaptureKind::Packet,
            direction: Some(direction),
            source_mac,
            destination_mac,
            source_ip,
            destination_ip,
            source_port,
            destination_port,
            ..CaptureRecord::EMPTY
        }, payload)
    }

    pub fn record_event(&mut self, timestamp_ms: u64, kind: CaptureKind, code: u16) {
        self.record(CaptureRecord {
            timestamp_ms,
            kind,
            code,
            ..CaptureRecord::EMPTY
        }, &[])
    }

    fn record(&mut self, mut record: CaptureRecord, payload: &[u8]) {
        if self.length == CAPACITY {
            self.dropped = self.dropped.saturating_add(1);
            return
        }
        record.sequence = self.next_sequence;
        record.original_length = payload.len();
        record.truncated = payload.len() > MAX_CAPTURE_BYTES;
        record.length = payload.len().min(MAX_CAPTURE_BYTES);
        record.payload[..record.length].copy_from_slice(&payload[..record.length]);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.records[self.length] = record;
        self.length += 1;
    }
}

impl<const CAPACITY: usize> Default for PacketCapture<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
