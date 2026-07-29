use crate::{Av1RtpPacketizer, DisplayError, EncodedFrame, EncoderConfig, RtpPacket};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct DisplayRights(u8);

impl DisplayRights {
    pub const VIEW: Self = Self(1 << 0);
    pub const SEND_INPUT: Self = Self(1 << 1);
    pub const ALL: Self = Self(Self::VIEW.0 | Self::SEND_INPUT.0);

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits != 0 && bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayGrant {
    pub principal: u64,
    pub display: u64,
    pub rights: DisplayRights,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebRtcOffer {
    pub av1: bool,
    pub av1_10_bit: bool,
    pub max_width: u16,
    pub max_height: u16,
    pub max_frames_per_second: u8,
    pub max_bitrate_bits_per_second: u32,
    pub mtu: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebRtcAnswer {
    pub config: EncoderConfig,
    pub payload_type: u8,
    pub ssrc: u32,
    pub mtu: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkFeedback {
    pub acknowledged_bitrate_bits_per_second: u32,
    pub round_trip_time_us: u32,
    pub lost_packets_per_mille: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct DisplaySessionId(u64);

impl DisplaySessionId {
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

#[derive(Clone, Copy)]
struct Session {
    generation: u32,
    active: bool,
    grant: Option<DisplayGrant>,
    answer: Option<WebRtcAnswer>,
    sequence: u16,
    force_keyframe: bool,
}

impl Session {
    const EMPTY: Self = Self {
        generation: 0,
        active: false,
        grant: None,
        answer: None,
        sequence: 0,
        force_keyframe: true,
    };
}

/// Capability gate and WebRTC media-session control plane.
pub struct DisplayDaemon<const SESSION_CAPACITY: usize> {
    sessions: [Session; SESSION_CAPACITY],
    next_ssrc: u32,
}

impl<const SESSION_CAPACITY: usize> DisplayDaemon<SESSION_CAPACITY> {
    pub const fn new(ssrc_seed: u32) -> Self {
        Self {
            sessions: [Session::EMPTY; SESSION_CAPACITY],
            next_ssrc: if ssrc_seed == 0 { 1 } else { ssrc_seed },
        }
    }

    pub fn negotiate(
        &mut self,
        grant: DisplayGrant,
        offer: WebRtcOffer,
        now_us: u64,
    ) -> Result<(DisplaySessionId, WebRtcAnswer), DisplayError> {
        if grant.principal == 0
            || grant.display == 0
            || !grant.rights.contains(DisplayRights::VIEW)
            || now_us >= grant.expires_at_us
        {
            return Err(DisplayError::AccessDenied);
        }
        if !offer.av1
            || offer.max_width == 0
            || offer.max_height == 0
            || offer.max_frames_per_second == 0
            || offer.mtu <= 13
        {
            return Err(DisplayError::NegotiationFailed);
        }
        let config = EncoderConfig {
            profile: if offer.av1_10_bit {
                crate::Av1Profile::Main10
            } else {
                crate::Av1Profile::Main8
            },
            width: offer.max_width,
            height: offer.max_height,
            frames_per_second: offer.max_frames_per_second.min(120),
            bitrate_bits_per_second: offer.max_bitrate_bits_per_second.max(64_000),
            keyframe_interval: (offer.max_frames_per_second as u16)
                .saturating_mul(2)
                .max(1),
            low_latency: true,
        }
        .validate()?;
        let slot_index = self
            .sessions
            .iter()
            .position(|session| !session.active)
            .ok_or(DisplayError::SessionCapacity)?;
        let answer = WebRtcAnswer {
            config,
            payload_type: 98,
            ssrc: self.allocate_ssrc(),
            mtu: offer.mtu,
        };
        let slot = &mut self.sessions[slot_index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.active = true;
        slot.grant = Some(grant);
        slot.answer = Some(answer);
        slot.sequence = 0;
        slot.force_keyframe = true;
        Ok((
            DisplaySessionId::from_parts(slot_index, slot.generation),
            answer,
        ))
    }

    pub fn packetize(
        &mut self,
        session: DisplaySessionId,
        frame: EncodedFrame,
        now_us: u64,
        output: &mut [RtpPacket],
    ) -> Result<usize, DisplayError> {
        let slot = self.session_mut(session)?;
        let grant = slot.grant.ok_or(DisplayError::InvalidSession)?;
        if now_us >= grant.expires_at_us {
            return Err(DisplayError::AccessDenied);
        }
        let answer = slot.answer.ok_or(DisplayError::InvalidSession)?;
        let packetizer =
            Av1RtpPacketizer::new(answer.mtu, answer.payload_type).map_err(DisplayError::from)?;
        let count = packetizer.packetize(frame, answer.ssrc, &mut slot.sequence, output)?;
        if frame.keyframe {
            slot.force_keyframe = false
        }
        Ok(count)
    }

    pub fn apply_feedback(
        &mut self,
        session: DisplaySessionId,
        feedback: NetworkFeedback,
    ) -> Result<EncoderConfig, DisplayError> {
        if feedback.lost_packets_per_mille > 1_000 {
            return Err(DisplayError::InvalidConfiguration);
        }
        let slot = self.session_mut(session)?;
        let mut answer = slot.answer.ok_or(DisplayError::InvalidSession)?;
        let acknowledged = feedback.acknowledged_bitrate_bits_per_second.max(64_000);
        let loss_scale = 1_000u32.saturating_sub(feedback.lost_packets_per_mille as u32);
        let target = acknowledged
            .saturating_mul(loss_scale)
            .checked_div(1_000)
            .unwrap_or(64_000)
            .max(64_000);
        answer.config.bitrate_bits_per_second =
            target.min(answer.config.bitrate_bits_per_second.saturating_mul(5) / 4);
        if feedback.lost_packets_per_mille >= 100 || feedback.round_trip_time_us >= 300_000 {
            slot.force_keyframe = true
        }
        slot.answer = Some(answer);
        Ok(answer.config)
    }

    pub fn should_force_keyframe(&self, session: DisplaySessionId) -> Result<bool, DisplayError> {
        Ok(self.session(session)?.force_keyframe)
    }

    pub fn authorize_input(
        &self,
        session: DisplaySessionId,
        now_us: u64,
    ) -> Result<u64, DisplayError> {
        let grant = self
            .session(session)?
            .grant
            .ok_or(DisplayError::InvalidSession)?;
        if now_us >= grant.expires_at_us || !grant.rights.contains(DisplayRights::SEND_INPUT) {
            return Err(DisplayError::AccessDenied);
        }
        Ok(grant.principal)
    }

    pub fn close(&mut self, session: DisplaySessionId) -> Result<(), DisplayError> {
        let slot = self.session_mut(session)?;
        slot.active = false;
        slot.grant = None;
        slot.answer = None;
        slot.force_keyframe = true;
        Ok(())
    }

    pub fn active_sessions(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.active)
            .count()
    }

    fn allocate_ssrc(&mut self) -> u32 {
        let ssrc = self.next_ssrc;
        self.next_ssrc = self.next_ssrc.wrapping_add(1).max(1);
        ssrc
    }

    fn session(&self, session: DisplaySessionId) -> Result<&Session, DisplayError> {
        let slot = self
            .sessions
            .get(session.slot())
            .ok_or(DisplayError::InvalidSession)?;
        if !slot.active || slot.generation != session.generation() {
            return Err(DisplayError::InvalidSession);
        }
        Ok(slot)
    }

    fn session_mut(&mut self, session: DisplaySessionId) -> Result<&mut Session, DisplayError> {
        let slot = self
            .sessions
            .get_mut(session.slot())
            .ok_or(DisplayError::InvalidSession)?;
        if !slot.active || slot.generation != session.generation() {
            return Err(DisplayError::InvalidSession);
        }
        Ok(slot)
    }
}
