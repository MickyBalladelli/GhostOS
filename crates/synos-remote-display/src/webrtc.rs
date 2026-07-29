use synos_platform_io::BufferDescriptor;

use crate::{DisplayError, EncodedFrame};

pub const RTP_HEADER_BYTES: u16 = 12;
pub const AV1_AGGREGATION_HEADER_BYTES: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtpPacket {
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub av1_aggregation_header: u8,
    pub payload: BufferDescriptor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtpPacketError {
    InvalidMtu,
    InvalidSsrc,
    OutputTooSmall,
    InvalidFrame,
}

/// RFC 9364 AV1 RTP scatter/gather packetizer.
pub struct Av1RtpPacketizer {
    mtu: u16,
    payload_type: u8,
}

impl Av1RtpPacketizer {
    pub const fn new(mtu: u16, payload_type: u8) -> Result<Self, RtpPacketError> {
        if mtu <= RTP_HEADER_BYTES + AV1_AGGREGATION_HEADER_BYTES || payload_type > 127 {
            Err(RtpPacketError::InvalidMtu)
        } else {
            Ok(Self { mtu, payload_type })
        }
    }

    pub const fn mtu(&self) -> u16 {
        self.mtu
    }

    pub fn packetize(
        &self,
        frame: EncodedFrame,
        ssrc: u32,
        next_sequence: &mut u16,
        output: &mut [RtpPacket],
    ) -> Result<usize, RtpPacketError> {
        let frame = frame.validate().map_err(|_| RtpPacketError::InvalidFrame)?;
        if ssrc == 0 {
            return Err(RtpPacketError::InvalidSsrc);
        }
        let payload_capacity = (self.mtu - RTP_HEADER_BYTES - AV1_AGGREGATION_HEADER_BYTES) as u32;
        let packet_count = frame.bytes.div_ceil(payload_capacity) as usize;
        if output.len() < packet_count {
            return Err(RtpPacketError::OutputTooSmall);
        }

        for (index, packet) in output.iter_mut().take(packet_count).enumerate() {
            let payload_offset = index as u32 * payload_capacity;
            let bytes = core::cmp::min(payload_capacity, frame.bytes - payload_offset);
            let first = index == 0;
            let last = index + 1 == packet_count;
            let mut aggregation = 0u8;
            if !first {
                aggregation |= 1 << 7
            }
            if !last {
                aggregation |= 1 << 6
            }
            if first && frame.keyframe {
                aggregation |= 1 << 3
            }
            *packet = RtpPacket {
                payload_type: self.payload_type,
                sequence: *next_sequence,
                timestamp: frame.timestamp_90khz,
                ssrc,
                marker: last,
                av1_aggregation_header: aggregation,
                payload: BufferDescriptor {
                    region: frame.buffer.region,
                    offset: frame.buffer.offset + payload_offset as u64,
                    length: bytes,
                    access: frame.buffer.access,
                },
            };
            *next_sequence = next_sequence.wrapping_add(1)
        }
        Ok(packet_count)
    }
}

impl From<RtpPacketError> for DisplayError {
    fn from(error: RtpPacketError) -> Self {
        match error {
            RtpPacketError::OutputTooSmall => Self::BufferTooSmall,
            RtpPacketError::InvalidFrame => Self::InvalidFrame,
            RtpPacketError::InvalidMtu | RtpPacketError::InvalidSsrc => Self::InvalidConfiguration,
        }
    }
}
