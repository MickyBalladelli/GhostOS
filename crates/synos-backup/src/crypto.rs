use core::fmt;

pub const DIGEST_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 12;
pub const TAG_BYTES: usize = 32;
pub const CHUNK_BYTES: usize = 4096;
pub const ENCRYPTED_CHUNK_OVERHEAD: usize = NONCE_BYTES + TAG_BYTES;
pub const MAX_ENCRYPTED_CHUNK_BYTES: usize = CHUNK_BYTES + ENCRYPTED_CHUNK_OVERHEAD;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Digest([u8; DIGEST_BYTES]);

impl Digest {
    pub const fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; DIGEST_BYTES] {
        self.0
    }

    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }

    pub fn hash(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EncryptionKey([u8; DIGEST_BYTES]);

impl fmt::Debug for EncryptionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptionKey(REDACTED)")
    }
}

impl EncryptionKey {
    pub const fn new(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn object_id(self, plaintext_digest: Digest) -> Digest {
        let mut material = [0; 33];
        material[..1].copy_from_slice(b"o");
        material[1..].copy_from_slice(plaintext_digest.as_bytes());
        Digest(hmac_sha256(&self.0, &material))
    }

    pub fn encrypt(
        self,
        plaintext_digest: Digest,
        plaintext: &[u8],
        output: &mut [u8],
    ) -> Result<usize, CryptoError> {
        if plaintext.len() > CHUNK_BYTES {
            return Err(CryptoError::ChunkTooLarge)
        }
        let required = plaintext.len().saturating_add(ENCRYPTED_CHUNK_OVERHEAD);
        if output.len() < required {
            return Err(CryptoError::BufferTooSmall { required })
        }
        let nonce = self.nonce(plaintext_digest);
        output[..NONCE_BYTES].copy_from_slice(&nonce);
        for (index, byte) in plaintext.iter().enumerate() {
            output[NONCE_BYTES + index] = *byte ^ self.keystream_byte(&nonce, index);
        }
        let tag_start = NONCE_BYTES + plaintext.len();
        let tag = hmac_sha256(&self.0, &output[..tag_start]);
        output[tag_start..tag_start + TAG_BYTES].copy_from_slice(&tag);
        Ok(required)
    }

    pub fn decrypt(
        self,
        object_id: Digest,
        encoded: &[u8],
        plaintext_digest: Digest,
        output: &mut [u8],
    ) -> Result<usize, CryptoError> {
        if encoded.len() < ENCRYPTED_CHUNK_OVERHEAD {
            return Err(CryptoError::InvalidLength)
        }
        let plaintext_len = encoded.len() - ENCRYPTED_CHUNK_OVERHEAD;
        if plaintext_len > CHUNK_BYTES || output.len() < plaintext_len {
            return Err(CryptoError::BufferTooSmall {
                required: plaintext_len,
            })
        }
        if self.object_id(plaintext_digest) != object_id {
            return Err(CryptoError::AuthenticationFailed)
        }
        let tag_start = NONCE_BYTES + plaintext_len;
        let expected = hmac_sha256(&self.0, &encoded[..tag_start]);
        let mut actual = [0; TAG_BYTES];
        actual.copy_from_slice(&encoded[tag_start..]);
        if !constant_time_equal(&expected, &actual) {
            return Err(CryptoError::AuthenticationFailed)
        }
        let mut nonce = [0; NONCE_BYTES];
        nonce.copy_from_slice(&encoded[..NONCE_BYTES]);
        if nonce != self.nonce(plaintext_digest) {
            return Err(CryptoError::AuthenticationFailed)
        }
        for (index, byte) in output[..plaintext_len].iter_mut().enumerate() {
            *byte = encoded[NONCE_BYTES + index] ^ self.keystream_byte(&nonce, index);
        }
        if Digest::hash(&output[..plaintext_len]) != plaintext_digest {
            return Err(CryptoError::AuthenticationFailed)
        }
        Ok(plaintext_len)
    }

    fn nonce(self, plaintext_digest: Digest) -> [u8; NONCE_BYTES] {
        let mut material = [0; 33];
        material[..1].copy_from_slice(b"n");
        material[1..].copy_from_slice(plaintext_digest.as_bytes());
        let digest = hmac_sha256(&self.0, &material);
        let mut nonce = [0; NONCE_BYTES];
        nonce.copy_from_slice(&digest[..NONCE_BYTES]);
        nonce
    }

    fn keystream_byte(self, nonce: &[u8; NONCE_BYTES], index: usize) -> u8 {
        let block = index / DIGEST_BYTES;
        let offset = index % DIGEST_BYTES;
        let mut material = [0; NONCE_BYTES + 8];
        material[..NONCE_BYTES].copy_from_slice(nonce);
        material[NONCE_BYTES..].copy_from_slice(&(block as u64).to_le_bytes());
        hmac_sha256(&self.0, &material)[offset]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CryptoError {
    BufferTooSmall { required: usize },
    ChunkTooLarge,
    InvalidLength,
    AuthenticationFailed,
}

fn hmac_sha256(key: &[u8; DIGEST_BYTES], message: &[u8]) -> [u8; DIGEST_BYTES] {
    let mut normalized = [0; 64];
    normalized[..DIGEST_BYTES].copy_from_slice(key);
    let mut inner = [0; 64 + MAX_ENCRYPTED_CHUNK_BYTES];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36;
    }
    inner[64..64 + message.len()].copy_from_slice(message);
    let inner_hash = sha256(&inner[..64 + message.len()]);
    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c;
    }
    outer[64..].copy_from_slice(&inner_hash);
    sha256(&outer)
}

fn constant_time_equal(left: &[u8; TAG_BYTES], right: &[u8; TAG_BYTES]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn sha256(message: &[u8]) -> [u8; DIGEST_BYTES] {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
        0x1f83d9ab, 0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
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
    let mut state = INITIAL;
    let full_blocks = message.len() / 64;
    for block in 0..full_blocks {
        compress(&mut state, &message[block * 64..block * 64 + 64], &ROUND)
    }
    let remainder = &message[full_blocks * 64..];
    let mut tail = [0u8; 128];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;
    let tail_length = if remainder.len() < 56 { 64 } else { 128 };
    tail[tail_length - 8..tail_length]
        .copy_from_slice(&(message.len() as u64).wrapping_mul(8).to_be_bytes());
    compress(&mut state, &tail[..64], &ROUND);
    if tail_length == 128 {
        compress(&mut state, &tail[64..], &ROUND)
    }
    let mut digest = [0; DIGEST_BYTES];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes())
    }
    digest
}

fn compress(state: &mut [u32; 8], block: &[u8], round: &[u32; 64]) {
    let mut schedule = [0u32; 64];
    for (word, bytes) in schedule.iter_mut().zip(block.chunks_exact(4)).take(16) {
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
        let choice = (e & f) ^ (!e & g);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let first = h
            .wrapping_add(sum1)
            .wrapping_add(choice)
            .wrapping_add(round[index])
            .wrapping_add(schedule[index]);
        let second = sum0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(first);
        d = c;
        c = b;
        b = a;
        a = first.wrapping_add(second)
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value)
    }
}
