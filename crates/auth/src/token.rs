use core::fmt;

use ghostos_fabric::NodeId;
use ghostos_kernel::Rights;

pub const MAX_CAPABILITY_CAVEATS: usize = 4;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CapabilityKey([u8; 32]);

impl fmt::Debug for CapabilityKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityKey")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl CapabilityKey {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn authenticate(self, message: &[u8]) -> Result<[u8; 32], TokenError> {
        if message.len() > 512 {
            return Err(TokenError::Invalid)
        }
        Ok(hmac_sha256(&self.0, message))
    }

    pub fn verify_authenticator(
        self,
        message: &[u8],
        authenticator: &[u8; 32],
    ) -> Result<(), TokenError> {
        let expected = self.authenticate(message)?;
        if constant_time_equal(&expected, authenticator) {
            Ok(())
        } else {
            Err(TokenError::InvalidSignature)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TransportRights(u8);

impl TransportRights {
    pub const CXL: Self = Self(1);
    pub const LAYER2: Self = Self(2);
    pub const ALL: Self = Self(Self::CXL.0 | Self::LAYER2.0);

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

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityCaveat {
    pub subject: Option<NodeId>,
    pub rights: Rights,
    pub transports: TransportRights,
    pub expires_at_us: u64,
}

/// HMAC-SHA256 capability with a fixed Macaroon-style attenuation chain.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CryptographicCapability {
    pub issuer: NodeId,
    pub subject: NodeId,
    pub resource: u64,
    pub rights: Rights,
    pub transports: TransportRights,
    pub not_before_us: u64,
    pub expires_at_us: u64,
    pub revocation_epoch: u64,
    pub nonce: u64,
    caveats: [Option<CapabilityCaveat>; MAX_CAPABILITY_CAVEATS],
    tag: [u8; 32],
}

impl fmt::Debug for CryptographicCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CryptographicCapability")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl CryptographicCapability {
    pub const WIRE_BYTES: usize = 192;

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        key: CapabilityKey,
        issuer: NodeId,
        subject: NodeId,
        resource: u64,
        rights: Rights,
        transports: TransportRights,
        not_before_us: u64,
        expires_at_us: u64,
        revocation_epoch: u64,
        nonce: u64,
    ) -> Result<Self, TokenError> {
        if resource == 0
            || rights.is_empty()
            || transports.0 == 0
            || expires_at_us <= not_before_us
            || revocation_epoch == 0
            || nonce == 0
        {
            return Err(TokenError::Invalid)
        }
        let mut token = Self {
            issuer,
            subject,
            resource,
            rights,
            transports,
            not_before_us,
            expires_at_us,
            revocation_epoch,
            nonce,
            caveats: [None; MAX_CAPABILITY_CAVEATS],
            tag: [0; 32],
        };
        token.tag = hmac_sha256(&key.0, &token.base_bytes());
        Ok(token)
    }

    pub fn caveats(&self) -> impl Iterator<Item = CapabilityCaveat> + '_ {
        self.caveats.iter().flatten().copied()
    }

    pub fn effective_rights(&self) -> Rights {
        self.caveats()
            .fold(self.rights, |rights, caveat| rights.intersection(caveat.rights))
    }

    pub fn effective_transports(&self) -> TransportRights {
        self.caveats().fold(self.transports, |transports, caveat| {
            transports.intersection(caveat.transports)
        })
    }

    pub fn effective_expiry(&self) -> u64 {
        self.caveats()
            .fold(self.expires_at_us, |expiry, caveat| {
                core::cmp::min(expiry, caveat.expires_at_us)
            })
    }

    pub fn effective_subject(&self) -> NodeId {
        self.caveats()
            .filter_map(|caveat| caveat.subject)
            .last()
            .unwrap_or(self.subject)
    }

    pub fn subject_is_sealed(&self) -> bool {
        self.caveats().any(|caveat| caveat.subject.is_some())
    }

    /// Add a restriction without possessing the issuer key.
    pub fn attenuate(mut self, caveat: CapabilityCaveat) -> Result<Self, TokenError> {
        if caveat.rights.is_empty()
            || !self.effective_rights().contains(caveat.rights)
            || !self
                .effective_transports()
                .contains(caveat.transports)
            || caveat.expires_at_us > self.effective_expiry()
            || (caveat.subject.is_some() && self.subject_is_sealed())
        {
            return Err(TokenError::RightsEscalation)
        }
        let slot = self
            .caveats
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(TokenError::CaveatCapacity)?;
        self.tag = hmac_sha256(&self.tag, &caveat_bytes(caveat));
        *slot = Some(caveat);
        Ok(self)
    }

    pub fn verify(
        &self,
        key: CapabilityKey,
        subject: NodeId,
        required: Rights,
        transport: TransportRights,
        now_us: u64,
        current_epoch: u64,
    ) -> Result<(), TokenError> {
        if !self.has_valid_caveat_chain()
            || subject != self.effective_subject()
            || !self.effective_rights().contains(required)
            || !self.effective_transports().contains(transport)
            || now_us < self.not_before_us
            || now_us >= self.effective_expiry()
            || current_epoch != self.revocation_epoch
        {
            return Err(TokenError::AccessDenied)
        }
        let mut expected = hmac_sha256(&key.0, &self.base_bytes());
        for caveat in self.caveats() {
            expected = hmac_sha256(&expected, &caveat_bytes(caveat))
        }
        if constant_time_equal(&expected, &self.tag) {
            Ok(())
        } else {
            Err(TokenError::InvalidSignature)
        }
    }

    fn has_valid_caveat_chain(&self) -> bool {
        let mut saw_empty = false;
        let mut subject_caveats = 0;
        let mut rights = self.rights;
        let mut transports = self.transports;
        let mut expiry = self.expires_at_us;
        for entry in self.caveats {
            let Some(caveat) = entry else {
                saw_empty = true;
                continue
            };
            if saw_empty
                || caveat.rights.is_empty()
                || !rights.contains(caveat.rights)
                || !transports.contains(caveat.transports)
                || caveat.expires_at_us > expiry
            {
                return false
            }
            if caveat.subject.is_some() {
                subject_caveats += 1;
                if subject_caveats > 1 {
                    return false
                }
            }
            rights = caveat.rights;
            transports = caveat.transports;
            expiry = caveat.expires_at_us;
        }
        true
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut output = [0; Self::WIRE_BYTES];
        output[0..4].copy_from_slice(b"SYCA");
        output[4] = 1;
        output[8..12].copy_from_slice(&self.issuer.raw().to_be_bytes());
        output[12..16].copy_from_slice(&self.subject.raw().to_be_bytes());
        output[16..24].copy_from_slice(&self.resource.to_be_bytes());
        output[24..26].copy_from_slice(&self.rights.bits().to_be_bytes());
        output[26] = self.transports.0;
        output[32..40].copy_from_slice(&self.not_before_us.to_be_bytes());
        output[40..48].copy_from_slice(&self.expires_at_us.to_be_bytes());
        output[48..56].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        output[56..64].copy_from_slice(&self.nonce.to_be_bytes());
        for (index, caveat) in self.caveats.iter().enumerate() {
            let offset = 64 + index * 24;
            if let Some(caveat) = caveat {
                output[offset] = 1;
                output[offset + 1] = caveat.transports.0;
                output[offset + 2..offset + 4]
                    .copy_from_slice(&caveat.rights.bits().to_be_bytes());
                output[offset + 4..offset + 8].copy_from_slice(
                    &caveat.subject.map(NodeId::raw).unwrap_or(0).to_be_bytes(),
                );
                output[offset + 8..offset + 16]
                    .copy_from_slice(&caveat.expires_at_us.to_be_bytes());
            }
        }
        output[160..192].copy_from_slice(&self.tag);
        output
    }

    pub fn decode(input: [u8; Self::WIRE_BYTES]) -> Result<Self, TokenError> {
        if &input[0..4] != b"SYCA" || input[4] != 1 {
            return Err(TokenError::Invalid)
        }
        let issuer = NodeId::new(read_u32(&input, 8)).ok_or(TokenError::Invalid)?;
        let subject = NodeId::new(read_u32(&input, 12)).ok_or(TokenError::Invalid)?;
        let rights = Rights::from_bits(read_u16(&input, 24)).ok_or(TokenError::Invalid)?;
        let transports = TransportRights::from_bits(input[26]).ok_or(TokenError::Invalid)?;
        let mut caveats = [None; MAX_CAPABILITY_CAVEATS];
        for (index, slot) in caveats.iter_mut().enumerate() {
            let offset = 64 + index * 24;
            match input[offset] {
                0 => continue,
                1 => {}
                _ => return Err(TokenError::Invalid),
            }
            let caveat_rights =
                Rights::from_bits(read_u16(&input, offset + 2)).ok_or(TokenError::Invalid)?;
            let raw_subject = read_u32(&input, offset + 4);
            *slot = Some(CapabilityCaveat {
                subject: if raw_subject == 0 {
                    None
                } else {
                    Some(NodeId::new(raw_subject).ok_or(TokenError::Invalid)?)
                },
                rights: caveat_rights,
                transports: TransportRights::from_bits(input[offset + 1])
                    .ok_or(TokenError::Invalid)?,
                expires_at_us: read_u64(&input, offset + 8),
            })
        }
        let mut tag = [0; 32];
        tag.copy_from_slice(&input[160..192]);
        let token = Self {
            issuer,
            subject,
            resource: read_u64(&input, 16),
            rights,
            transports,
            not_before_us: read_u64(&input, 32),
            expires_at_us: read_u64(&input, 40),
            revocation_epoch: read_u64(&input, 48),
            nonce: read_u64(&input, 56),
            caveats,
            tag,
        };
        if token.resource == 0
            || token.rights.is_empty()
            || token.expires_at_us <= token.not_before_us
            || token.revocation_epoch == 0
            || token.nonce == 0
            || !token.has_valid_caveat_chain()
        {
            return Err(TokenError::Invalid)
        }
        Ok(token)
    }

    fn base_bytes(&self) -> [u8; 64] {
        let mut bytes = [0; 64];
        bytes[0..4].copy_from_slice(b"SYCA");
        bytes[4..8].copy_from_slice(&self.issuer.raw().to_be_bytes());
        bytes[8..12].copy_from_slice(&self.subject.raw().to_be_bytes());
        bytes[12..20].copy_from_slice(&self.resource.to_be_bytes());
        bytes[20..22].copy_from_slice(&self.rights.bits().to_be_bytes());
        bytes[22] = self.transports.0;
        bytes[24..32].copy_from_slice(&self.not_before_us.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        bytes[48..56].copy_from_slice(&self.nonce.to_be_bytes());
        bytes
    }
}

fn caveat_bytes(caveat: CapabilityCaveat) -> [u8; 24] {
    let mut bytes = [0; 24];
    bytes[0..4].copy_from_slice(
        &caveat
            .subject
            .map(NodeId::raw)
            .unwrap_or(0)
            .to_be_bytes(),
    );
    bytes[4..6].copy_from_slice(&caveat.rights.bits().to_be_bytes());
    bytes[6] = caveat.transports.0;
    bytes[8..16].copy_from_slice(&caveat.expires_at_us.to_be_bytes());
    bytes
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0; 64];
    if key.len() > normalized.len() {
        normalized[..32].copy_from_slice(&sha256(key))
    } else {
        normalized[..key.len()].copy_from_slice(key)
    }
    let mut inner = [0; 64 + 512];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36
    }
    inner[64..64 + message.len()].copy_from_slice(message);
    let inner_hash = sha256(&inner[..64 + message.len()]);
    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c
    }
    outer[64..].copy_from_slice(&inner_hash);
    sha256(&outer)
}

fn sha256(message: &[u8]) -> [u8; 32] {
    let mut state = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_length = (message.len() as u64).wrapping_mul(8);
    let blocks = (message.len() + 9).div_ceil(64);
    for block_index in 0..blocks {
        let mut block = [0_u8; 64];
        let start = block_index * 64;
        let copied = core::cmp::min(64, message.len().saturating_sub(start));
        if copied != 0 {
            block[..copied].copy_from_slice(&message[start..start + copied])
        }
        if copied < 64 && start + copied == message.len() {
            block[copied] = 0x80
        }
        if block_index + 1 == blocks {
            block[56..64].copy_from_slice(&bit_length.to_be_bytes())
        }
        compress(&mut state, &block)
    }
    let mut digest = [0; 32];
    for (chunk, word) in digest.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes())
    }
    digest
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
        0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut schedule = [0_u32; 64];
    for (word, bytes) in schedule.iter_mut().take(16).zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    for index in 16..64 {
        let s0 = schedule[index - 15].rotate_right(7)
            ^ schedule[index - 15].rotate_right(18)
            ^ (schedule[index - 15] >> 3);
        let s1 = schedule[index - 2].rotate_right(17)
            ^ schedule[index - 2].rotate_right(19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1)
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(K[index])
            .wrapping_add(schedule[index]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2)
    }
    for (value, add) in state
        .iter_mut()
        .zip([a, b, c, d, e, f, g, h])
    {
        *value = value.wrapping_add(add)
    }
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([input[offset], input[offset + 1]])
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenError {
    AccessDenied,
    CaveatCapacity,
    Invalid,
    InvalidSignature,
    RightsEscalation,
}
