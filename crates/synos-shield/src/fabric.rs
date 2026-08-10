use synos_fabric::{NodeId, PageFault, PAGE_SIZE};
use synos_fabric::dsm::{DsmPacket, FRAME_DATA_BYTES};
use synos_observability::{EventField, Level, audit_event, field};
use synos_status::IntoStatus;

use crate::{DIGEST_BYTES, Error, constant_time_equal, hmac_sha256};

const FRAME_BYTES: usize = 32 + FRAME_DATA_BYTES;
const SIGNED_FRAME_OVERHEAD: usize = DIGEST_BYTES;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FrameKey([u8; DIGEST_BYTES]);

impl FrameKey {
    pub const fn new(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn key_id(self) -> [u8; 16] {
        let digest = synos_system_model::ContentId::hash(&self.0);
        let mut id = [0; 16];
        id.copy_from_slice(&digest.as_bytes()[..16]);
        id
    }

    fn sign(self, packet: &DsmPacket) -> Result<[u8; DIGEST_BYTES], Error> {
        let mut encoded = [0; FRAME_BYTES];
        let length = packet.encode(&mut encoded).map_err(|_| Error::InvalidInput)?;
        Ok(hmac_sha256(&self.0, &encoded[..length]))
    }
}

#[derive(Clone, Copy)]
pub struct SignedFrame {
    pub packet: DsmPacket,
    pub signature: [u8; DIGEST_BYTES],
}

impl SignedFrame {
    pub fn new(packet: DsmPacket, key: FrameKey) -> Result<Self, Error> {
        Ok(Self {
            packet,
            signature: key.sign(&packet)?,
        })
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, Error> {
        let packet_length = self
            .packet
            .encode(output)
            .map_err(|_| Error::InvalidInput)?;
        if output.len() < packet_length + SIGNED_FRAME_OVERHEAD {
            return Err(Error::BufferTooSmall {
                required: packet_length + SIGNED_FRAME_OVERHEAD,
            })
        }
        output[packet_length..packet_length + DIGEST_BYTES].copy_from_slice(&self.signature);
        Ok(packet_length + DIGEST_BYTES)
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() <= DIGEST_BYTES {
            return Err(Error::InvalidInput)
        }
        let packet_length = input.len() - DIGEST_BYTES;
        let packet = DsmPacket::decode(&input[..packet_length]).map_err(|_| Error::InvalidInput)?;
        let mut signature = [0; DIGEST_BYTES];
        signature.copy_from_slice(&input[packet_length..]);
        Ok(Self { packet, signature })
    }

    pub fn verify(&self, key: FrameKey) -> Result<(), Error> {
        let expected = key.sign(&self.packet)?;
        if constant_time_equal(&expected, &self.signature) {
            Ok(())
        } else {
            Err(Error::SignatureMismatch)
        }
    }
}

#[derive(Clone, Copy)]
struct Peer {
    node: NodeId,
    key: FrameKey,
    last_sequence: Option<u32>,
}

pub struct FrameVerifier<const PEERS: usize = 32> {
    peers: [Option<Peer>; PEERS],
}

impl<const PEERS: usize> FrameVerifier<PEERS> {
    pub const fn new() -> Self {
        Self {
            peers: [None; PEERS],
        }
    }

    pub fn trust_peer(&mut self, node: NodeId, key: FrameKey) -> Result<(), Error> {
        if let Some(peer) = self.peers.iter_mut().flatten().find(|peer| peer.node == node) {
            peer.key = key;
            peer.last_sequence = None;
            return Ok(())
        }
        let slot = self
            .peers
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(Peer {
            node,
            key,
            last_sequence: None,
        });
        Ok(())
    }

    pub fn revoke_peer(&mut self, node: NodeId) -> Result<(), Error> {
        let peer = self
            .peers
            .iter_mut()
            .find(|entry| entry.is_some_and(|peer| peer.node == node))
            .ok_or(Error::Unauthorized)?;
        *peer = None;
        Ok(())
    }

    pub fn verify(
        &mut self,
        frame: &SignedFrame,
        destination: NodeId,
        allowed_memory: synos_fabric::AddressRange,
    ) -> Result<(), Error> {
        let header = frame.packet.header;
        let page_end = header.page_address.checked_add(PAGE_SIZE).ok_or(Error::Unauthorized)?;
        if header.destination != destination
            || !allowed_memory.contains(header.page_address)
            || page_end > allowed_memory.end()
        {
            return Err(Error::Unauthorized)
        }
        let peer = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.node == header.source)
            .ok_or(Error::Unauthorized)?;
        frame.verify(peer.key)?;
        if peer
            .last_sequence
            .is_some_and(|last| header.sequence <= last)
        {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, header.source.raw() as u64),
                EventField::unsigned(field::OPERATION, header.sequence as u64),
                EventField::status(Error::Replay.status()),
            );
            return Err(Error::Replay)
        }
        peer.last_sequence = Some(header.sequence);
        Ok(())
    }
}

impl<const PEERS: usize> Default for FrameVerifier<PEERS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultBudget {
    pub window_us: u64,
    pub max_faults: u32,
    pub cooldown_us: u64,
}

impl FaultBudget {
    pub const fn new(window_us: u64, max_faults: u32, cooldown_us: u64) -> Option<Self> {
        if window_us == 0 || max_faults == 0 || cooldown_us == 0 {
            None
        } else {
            Some(Self {
                window_us,
                max_faults,
                cooldown_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultDecision {
    Allowed,
    Throttled { retry_after_us: u64 },
}

#[derive(Clone, Copy)]
struct FaultWindow {
    subject: u64,
    started_at_us: u64,
    faults: u32,
    blocked_until_us: u64,
}

impl FaultWindow {
    const EMPTY: Self = Self {
        subject: 0,
        started_at_us: 0,
        faults: 0,
        blocked_until_us: 0,
    };
}

pub struct PageFaultLimiter<const SUBJECTS: usize = 64> {
    budget: FaultBudget,
    windows: [FaultWindow; SUBJECTS],
}

impl<const SUBJECTS: usize> PageFaultLimiter<SUBJECTS> {
    pub fn new(budget: FaultBudget) -> Result<Self, Error> {
        if SUBJECTS == 0 {
            return Err(Error::InvalidConfiguration)
        }
        Ok(Self {
            budget,
            windows: [FaultWindow::EMPTY; SUBJECTS],
        })
    }

    pub fn record(
        &mut self,
        subject: u64,
        now_us: u64,
        _fault: PageFault,
    ) -> Result<FaultDecision, Error> {
        if subject == 0 {
            return Err(Error::InvalidInput)
        }
        let window = self
            .windows
            .iter_mut()
            .find(|entry| entry.subject == subject || entry.subject == 0)
            .ok_or(Error::Capacity)?;
        if now_us < window.blocked_until_us {
            return Ok(FaultDecision::Throttled {
                retry_after_us: window.blocked_until_us - now_us,
            })
        }
        if window.subject == 0
            || now_us.saturating_sub(window.started_at_us) >= self.budget.window_us
        {
            *window = FaultWindow {
                subject,
                started_at_us: now_us,
                faults: 0,
                blocked_until_us: 0,
            }
        }
        if window.faults >= self.budget.max_faults {
            window.blocked_until_us = now_us.saturating_add(self.budget.cooldown_us);
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, subject),
                EventField::status(Error::FaultRateExceeded.status()),
            );
            return Ok(FaultDecision::Throttled {
                retry_after_us: self.budget.cooldown_us,
            })
        }
        window.faults += 1;
        Ok(FaultDecision::Allowed)
    }
}

impl<const SUBJECTS: usize> Default for PageFaultLimiter<SUBJECTS> {
    fn default() -> Self {
        Self::new(FaultBudget {
            window_us: 1_000_000,
            max_faults: 128,
            cooldown_us: 5_000_000,
        })
        .expect("valid default fault budget")
    }
}
