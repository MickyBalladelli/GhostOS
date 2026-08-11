//! Bounded packet and control-plane evidence for deterministic network tests.

use crate::dhcp::MAX_DHCP_PACKET;

pub const MAX_CAPTURE_RECORDS: usize = 32;
pub const MAX_CAPTURE_BYTES: usize = MAX_DHCP_PACKET;
pub const MAX_CAPTURE_FLOWS: usize = 32;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureFlowAggregate {
    pub tenant: u64,
    pub direction: CaptureDirection,
    pub source_ip: [u8; 4],
    pub destination_ip: [u8; 4],
    pub source_port: u16,
    pub destination_port: u16,
    pub packets: u64,
    pub bytes: u64,
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
    flows: [Option<CaptureFlowAggregate>; MAX_CAPTURE_FLOWS],
    length: usize,
    next_sequence: u64,
    dropped: u64,
    flow_cardinality_dropped: u64,
    other_flow: CaptureFlowAggregate,
}

impl<const CAPACITY: usize> PacketCapture<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [CaptureRecord::EMPTY; CAPACITY],
            flows: [None; MAX_CAPTURE_FLOWS],
            length: 0,
            next_sequence: 0,
            dropped: 0,
            flow_cardinality_dropped: 0,
            other_flow: CaptureFlowAggregate {
                tenant: 0,
                direction: CaptureDirection::Ingress,
                source_ip: [0; 4],
                destination_ip: [0; 4],
                source_port: 0,
                destination_port: 0,
                packets: 0,
                bytes: 0,
            },
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

    pub const fn flow_cardinality_dropped(&self) -> u64 {
        self.flow_cardinality_dropped
    }

    pub fn records(&self) -> &[CaptureRecord] {
        &self.records[..self.length]
    }

    pub fn flow_aggregates(&self) -> impl Iterator<Item = CaptureFlowAggregate> + '_ {
        self.flows.iter().flatten().copied()
    }

    pub const fn other_flow(&self) -> CaptureFlowAggregate {
        self.other_flow
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
        self.record_packet_for_tenant(
            1,
            timestamp_ms,
            direction,
            source_mac,
            destination_mac,
            source_ip,
            destination_ip,
            source_port,
            destination_port,
            payload,
        )
    }

    pub fn record_packet_for_tenant(
        &mut self,
        tenant: u64,
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
        let record = CaptureRecord {
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
        };
        self.record(tenant.max(1), record, payload)
    }

    pub fn record_event(&mut self, timestamp_ms: u64, kind: CaptureKind, code: u16) {
        self.record(1, CaptureRecord {
            timestamp_ms,
            kind,
            code,
            ..CaptureRecord::EMPTY
        }, &[])
    }

    fn record(&mut self, tenant: u64, mut record: CaptureRecord, payload: &[u8]) {
        if let (CaptureKind::Packet, Some(direction)) = (record.kind, record.direction) {
            self.aggregate_flow(CaptureFlowAggregate {
                tenant,
                direction,
                source_ip: record.source_ip,
                destination_ip: record.destination_ip,
                source_port: record.source_port,
                destination_port: record.destination_port,
                packets: 1,
                bytes: payload.len() as u64,
            });
        }
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

    fn aggregate_flow(&mut self, sample: CaptureFlowAggregate) {
        if let Some(existing) = self.flows.iter_mut().flatten().find(|entry| {
            entry.tenant == sample.tenant
                && entry.direction == sample.direction
                && entry.source_ip == sample.source_ip
                && entry.destination_ip == sample.destination_ip
                && entry.source_port == sample.source_port
                && entry.destination_port == sample.destination_port
        }) {
            existing.packets = existing.packets.saturating_add(sample.packets);
            existing.bytes = existing.bytes.saturating_add(sample.bytes);
            return
        }
        if let Some(slot) = self.flows.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(sample);
        } else {
            self.flow_cardinality_dropped = self.flow_cardinality_dropped.saturating_add(1);
            self.other_flow.packets = self.other_flow.packets.saturating_add(sample.packets);
            self.other_flow.bytes = self.other_flow.bytes.saturating_add(sample.bytes);
        }
    }
}

impl<const CAPACITY: usize> Default for PacketCapture<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
