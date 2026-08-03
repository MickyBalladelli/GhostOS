use synos_fabric::dsm::{DsmPacket, FRAME_DATA_BYTES};
use synos_system_model::ContentId;

use crate::Error;

pub const MAX_PACKET_BYTES: usize = 32 + FRAME_DATA_BYTES;
pub const ML_KEM_KEY_BYTES: usize = 32;
pub const ML_KEM_CIPHERTEXT_BYTES: usize = 32;
pub const FABRIC_NONCE_BYTES: usize = 16;
pub const FABRIC_TAG_BYTES: usize = 32;
pub const ENCRYPTED_FRAME_BYTES: usize =
    4 + 1 + 1 + ML_KEM_CIPHERTEXT_BYTES + FABRIC_NONCE_BYTES + 2 + MAX_PACKET_BYTES
        + FABRIC_TAG_BYTES;

const FRAME_MAGIC: [u8; 4] = *b"SCF1";
pub const DEFAULT_NONCE_HISTORY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FabricTransport {
    Cxl,
    Ethernet,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MlKemPublicKey([u8; ML_KEM_KEY_BYTES]);

impl MlKemPublicKey {
    pub fn from_bytes(bytes: [u8; ML_KEM_KEY_BYTES]) -> Option<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            None
        } else {
            Some(Self(bytes))
        }
    }

    pub const fn as_bytes(self) -> [u8; ML_KEM_KEY_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MlKemSecretKey([u8; ML_KEM_KEY_BYTES]);

impl MlKemSecretKey {
    pub fn from_seed(seed: [u8; ML_KEM_KEY_BYTES]) -> Self {
        let mut material = [0; ML_KEM_KEY_BYTES + 9];
        material[..9].copy_from_slice(b"synos-kem");
        material[9..].copy_from_slice(&seed);
        Self(*ContentId::hash(&material).as_bytes())
    }

    pub const fn as_bytes(self) -> [u8; ML_KEM_KEY_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MlKemCiphertext([u8; ML_KEM_CIPHERTEXT_BYTES]);

impl MlKemCiphertext {
    pub fn from_bytes(bytes: [u8; ML_KEM_CIPHERTEXT_BYTES]) -> Option<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            None
        } else {
            Some(Self(bytes))
        }
    }

    pub const fn as_bytes(self) -> [u8; ML_KEM_CIPHERTEXT_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SharedSecret([u8; ML_KEM_KEY_BYTES]);

impl SharedSecret {
    pub fn from_bytes(bytes: [u8; ML_KEM_KEY_BYTES]) -> Option<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            None
        } else {
            Some(Self(bytes))
        }
    }

    pub const fn as_bytes(self) -> [u8; ML_KEM_KEY_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MlKemKeypair {
    public: MlKemPublicKey,
    secret: MlKemSecretKey,
}

impl MlKemKeypair {
    pub fn from_seed(seed: [u8; ML_KEM_KEY_BYTES]) -> Self {
        let secret = MlKemSecretKey::from_seed(seed);
        let public = MlKemPublicKey::from_bytes(*ContentId::hash(&secret.0).as_bytes())
            .expect("hashed KEM key is non-zero");
        Self { public, secret }
    }

    pub const fn public_key(self) -> MlKemPublicKey {
        self.public
    }

    pub const fn secret_key(self) -> MlKemSecretKey {
        self.secret
    }
}

/// Portable fixed-size KEM seam used by the kernel until a platform ML-KEM
/// implementation is wired in. The wire contract is ML-KEM-shaped: the
/// platform adapter owns the real ML-KEM-768 primitive and may replace these
/// two functions without changing confidential capability or fabric code.
pub fn encapsulate(
    public: MlKemPublicKey,
    ephemeral: [u8; ML_KEM_CIPHERTEXT_BYTES],
) -> Result<(MlKemCiphertext, SharedSecret), Error> {
    let ciphertext = MlKemCiphertext::from_bytes(ephemeral).ok_or(Error::InvalidInput)?;
    Ok((ciphertext, derive_shared(public, ciphertext)))
}

pub fn decapsulate(
    secret: MlKemSecretKey,
    ciphertext: MlKemCiphertext,
) -> Result<SharedSecret, Error> {
    let public = MlKemPublicKey::from_bytes(*ContentId::hash(&secret.0).as_bytes())
        .ok_or(Error::InvalidInput)?;
    Ok(derive_shared(public, ciphertext))
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EncryptedDsmFrame {
    transport: FabricTransport,
    kem_ciphertext: MlKemCiphertext,
    nonce: [u8; FABRIC_NONCE_BYTES],
    ciphertext: [u8; MAX_PACKET_BYTES],
    length: u16,
    tag: [u8; FABRIC_TAG_BYTES],
}

impl EncryptedDsmFrame {
    pub fn seal(
        packet: &DsmPacket,
        transport: FabricTransport,
        recipient: MlKemPublicKey,
        ephemeral: [u8; ML_KEM_CIPHERTEXT_BYTES],
        nonce: [u8; FABRIC_NONCE_BYTES],
    ) -> Result<Self, Error> {
        let (kem_ciphertext, shared) = encapsulate(recipient, ephemeral)?;
        Self::seal_with_session(packet, transport, kem_ciphertext, shared, nonce)
    }

    pub fn seal_with_session(
        packet: &DsmPacket,
        transport: FabricTransport,
        kem_ciphertext: MlKemCiphertext,
        shared: SharedSecret,
        nonce: [u8; FABRIC_NONCE_BYTES],
    ) -> Result<Self, Error> {
        let mut plaintext = [0; MAX_PACKET_BYTES];
        let length = packet
            .encode(&mut plaintext)
            .map_err(|_| Error::InvalidInput)?;
        let mut ciphertext = [0; MAX_PACKET_BYTES];
        xor_stream(
            &mut ciphertext[..length],
            &plaintext[..length],
            shared,
            nonce,
        );
        let tag = authentication_tag(
            transport,
            kem_ciphertext,
            nonce,
            &ciphertext[..length],
            shared,
        );
        Ok(Self {
            transport,
            kem_ciphertext,
            nonce,
            ciphertext,
            length: length as u16,
            tag,
        })
    }

    pub fn open(&self, secret: MlKemSecretKey) -> Result<DsmPacket, Error> {
        let shared = decapsulate(secret, self.kem_ciphertext)?;
        self.open_with_session(shared)
    }

    pub fn open_with_session(&self, shared: SharedSecret) -> Result<DsmPacket, Error> {
        let length = self.length as usize;
        if length == 0 || length > MAX_PACKET_BYTES {
            return Err(Error::InvalidInput)
        }
        let expected = authentication_tag(
            self.transport,
            self.kem_ciphertext,
            self.nonce,
            &self.ciphertext[..length],
            shared,
        );
        if !constant_time_equal(&expected, &self.tag) {
            return Err(Error::AuthenticationFailed)
        }
        let mut plaintext = [0; MAX_PACKET_BYTES];
        xor_stream(
            &mut plaintext[..length],
            &self.ciphertext[..length],
            shared,
            self.nonce,
        );
        DsmPacket::decode(&plaintext[..length]).map_err(|_| Error::AuthenticationFailed)
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, Error> {
        if output.len() < ENCRYPTED_FRAME_BYTES {
            return Err(Error::InvalidInput)
        }
        output[..ENCRYPTED_FRAME_BYTES].fill(0);
        output[..4].copy_from_slice(&FRAME_MAGIC);
        output[4] = 1;
        output[5] = match self.transport {
            FabricTransport::Cxl => 1,
            FabricTransport::Ethernet => 2,
        };
        output[6..38].copy_from_slice(&self.kem_ciphertext.0);
        output[38..54].copy_from_slice(&self.nonce);
        output[54..56].copy_from_slice(&self.length.to_be_bytes());
        output[56..56 + MAX_PACKET_BYTES].copy_from_slice(&self.ciphertext);
        output[56 + MAX_PACKET_BYTES..].copy_from_slice(&self.tag);
        Ok(ENCRYPTED_FRAME_BYTES)
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != ENCRYPTED_FRAME_BYTES
            || input[..4] != FRAME_MAGIC
            || input[4] != 1
        {
            return Err(Error::InvalidInput)
        }
        let transport = match input[5] {
            1 => FabricTransport::Cxl,
            2 => FabricTransport::Ethernet,
            _ => return Err(Error::InvalidInput),
        };
        let mut kem_bytes = [0; ML_KEM_CIPHERTEXT_BYTES];
        kem_bytes.copy_from_slice(&input[6..38]);
        let kem_ciphertext = MlKemCiphertext::from_bytes(kem_bytes).ok_or(Error::InvalidInput)?;
        let mut nonce = [0; FABRIC_NONCE_BYTES];
        nonce.copy_from_slice(&input[38..54]);
        let length = u16::from_be_bytes([input[54], input[55]]) as usize;
        if length == 0 || length > MAX_PACKET_BYTES {
            return Err(Error::InvalidInput)
        }
        let mut ciphertext = [0; MAX_PACKET_BYTES];
        ciphertext.copy_from_slice(&input[56..56 + MAX_PACKET_BYTES]);
        let mut tag = [0; FABRIC_TAG_BYTES];
        tag.copy_from_slice(&input[56 + MAX_PACKET_BYTES..]);
        Ok(Self {
            transport,
            kem_ciphertext,
            nonce,
            ciphertext,
            length: length as u16,
            tag,
        })
    }

    pub const fn transport(&self) -> FabricTransport {
        self.transport
    }

    pub const fn kem_ciphertext(&self) -> MlKemCiphertext {
        self.kem_ciphertext
    }

    pub const fn nonce(&self) -> [u8; FABRIC_NONCE_BYTES] {
        self.nonce
    }

    pub const fn ciphertext_len(&self) -> usize {
        self.length as usize
    }
}

/// Receiver-side replay fence for encrypted frames. The DSM packet sequence
/// remains authoritative; this bounded nonce history stops a captured frame
/// from being accepted twice before the DSM sequence verifier runs.
pub struct NonceReplayGuard<const CAPACITY: usize = DEFAULT_NONCE_HISTORY> {
    nonces: [Option<[u8; FABRIC_NONCE_BYTES]>; CAPACITY],
}

impl<const CAPACITY: usize> NonceReplayGuard<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            nonces: [None; CAPACITY],
        }
    }

    pub fn accept(&mut self, frame: &EncryptedDsmFrame) -> Result<(), Error> {
        if CAPACITY == 0 {
            return Err(Error::InvalidConfiguration)
        }
        if self
            .nonces
            .iter()
            .flatten()
            .any(|nonce| *nonce == frame.nonce)
        {
            return Err(Error::Replay)
        }
        let slot = self
            .nonces
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(frame.nonce);
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for NonceReplayGuard<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn derive_shared(public: MlKemPublicKey, ciphertext: MlKemCiphertext) -> SharedSecret {
    let mut material = [0; 8 + ML_KEM_KEY_BYTES + ML_KEM_CIPHERTEXT_BYTES];
    material[..8].copy_from_slice(b"synos-ss");
    material[8..40].copy_from_slice(&public.0);
    material[40..].copy_from_slice(&ciphertext.0);
    SharedSecret(*ContentId::hash(&material).as_bytes())
}

fn xor_stream(
    output: &mut [u8],
    input: &[u8],
    shared: SharedSecret,
    nonce: [u8; FABRIC_NONCE_BYTES],
) {
    for (block, chunk) in input.chunks(ML_KEM_KEY_BYTES).enumerate() {
        let mut material = [0; 9 + ML_KEM_KEY_BYTES + FABRIC_NONCE_BYTES + 8];
        material[..9].copy_from_slice(b"synos-ctr");
        material[9..41].copy_from_slice(&shared.0);
        material[41..57].copy_from_slice(&nonce);
        material[57..].copy_from_slice(&(block as u64).to_be_bytes());
        let stream = ContentId::hash(&material);
        let offset = block * ML_KEM_KEY_BYTES;
        for (index, byte) in chunk.iter().enumerate() {
            output[offset + index] = *byte ^ stream.as_bytes()[index]
        }
    }
}

fn authentication_tag(
    transport: FabricTransport,
    kem_ciphertext: MlKemCiphertext,
    nonce: [u8; FABRIC_NONCE_BYTES],
    ciphertext: &[u8],
    shared: SharedSecret,
) -> [u8; FABRIC_TAG_BYTES] {
    let mut material = [0; 9 + ML_KEM_KEY_BYTES + FABRIC_NONCE_BYTES + 1 + ML_KEM_CIPHERTEXT_BYTES + 8 + MAX_PACKET_BYTES];
    material[..9].copy_from_slice(b"synos-tag");
    material[9..41].copy_from_slice(&shared.0);
    material[41..57].copy_from_slice(&nonce);
    material[57] = match transport {
        FabricTransport::Cxl => 1,
        FabricTransport::Ethernet => 2,
    };
    material[58..90].copy_from_slice(&kem_ciphertext.0);
    material[90..98].copy_from_slice(&(ciphertext.len() as u64).to_be_bytes());
    material[98..98 + ciphertext.len()].copy_from_slice(ciphertext);
    *ContentId::hash(&material[..98 + ciphertext.len()]).as_bytes()
}

fn constant_time_equal(left: &[u8; FABRIC_TAG_BYTES], right: &[u8; FABRIC_TAG_BYTES]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}
