#![cfg_attr(
    not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))),
    allow(dead_code)
)]
use core::cmp::Ordering;

pub const RP_ID: &str = "localhost";
pub const ORIGIN: &str = "http://localhost";

const HEADER_BYTES: usize = 12;
const MAX_AUTHENTICATOR_DATA: usize = 256;
const MAX_CLIENT_DATA: usize = 1024;
const MAX_SIGNATURE: usize = 128;
const AUTHENTICATOR_DATA_MIN: usize = 37;
const FLAG_USER_PRESENT: u8 = 1;
const FLAG_USER_VERIFIED: u8 = 1 << 2;

type U256 = [u64; 4];

const ZERO: U256 = [0; 4];
const ONE: U256 = [1, 0, 0, 0];
const FIELD_MODULUS: U256 = [
    0xffff_ffff_ffff_ffff,
    0x0000_0000_ffff_ffff,
    0,
    0xffff_ffff_0000_0001,
];
const ORDER: U256 = [
    0xf3b9_cac2_fc63_2551,
    0xbce6_faad_a717_9e84,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_0000_0000,
];
const CURVE_B: U256 = [
    0x3bce_3c3e_27d2_604b,
    0x651d_06b0_cc53_b0f6,
    0xb3eb_bd55_7698_86bc,
    0x5ac6_35d8_aa3a_93e7,
];
const GENERATOR_X: U256 = [
    0xf4a1_3945_d898_c296,
    0x7703_7d81_2deb_33a0,
    0xf8bc_e6e5_63a4_40f2,
    0x6b17_d1f2_e12c_4247,
];
const GENERATOR_Y: U256 = [
    0xcbb6_4068_37bf_51f5,
    0x2bce_3357_6b31_5ece,
    0x8ee7_eb4a_7c0f_9e16,
    0x4fe3_42e2_fe1a_7f9b,
];

#[derive(Clone, Copy)]
struct Point {
    x: U256,
    y: U256,
    z: U256,
}

impl Point {
    const fn infinity() -> Self {
        Self {
            x: ZERO,
            y: ONE,
            z: ZERO,
        }
    }

    const fn generator() -> Self {
        Self {
            x: GENERATOR_X,
            y: GENERATOR_Y,
            z: ONE,
        }
    }

    fn is_infinity(self) -> bool {
        self.z == ZERO
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub fn verify_local_assertion(
    assertion: &[u8],
    username: &[u8],
    challenge: &[u8; 32],
) -> Option<(usize, [u8; 32], u32)> {
    let (authenticator_data, client_data_json, signature) = parse_assertion(assertion)?;
    if !verify_client_data(client_data_json, challenge)
        || !verify_authenticator_data(authenticator_data)
    {
        return None
    }
    let client_data_hash = sha256(client_data_json);
    let mut signed_data = [0; MAX_AUTHENTICATOR_DATA + 32];
    if authenticator_data.len() + client_data_hash.len() > signed_data.len() {
        return None
    }
    signed_data[..authenticator_data.len()].copy_from_slice(authenticator_data);
    signed_data[authenticator_data.len()..authenticator_data.len() + 32]
        .copy_from_slice(&client_data_hash);
    let digest = sha256(&signed_data[..authenticator_data.len() + 32]);
    let sign_count = u32::from_be_bytes([
        authenticator_data[33],
        authenticator_data[34],
        authenticator_data[35],
        authenticator_data[36],
    ]);

    let mut keys = [[0; crate::boot_services::LOCAL_PASSKEY_MAX_KEY_BYTES]; crate::boot_services::LOCAL_PASSKEY_MAX_KEYS];
    let mut lengths = [0; crate::boot_services::LOCAL_PASSKEY_MAX_KEYS];
    let key_count = crate::boot_services::local_passkey_keys(username, &mut keys, &mut lengths).ok()?;
    for index in 0..key_count {
        if verify_cose_key(
            &keys[index][..lengths[index] as usize],
            &digest,
            signature,
        ) {
            return Some((index, sha256(&keys[index][..lengths[index] as usize]), sign_count))
        }
    }
    None
}

pub fn valid_cose_es256_public_key(key: &[u8]) -> bool {
    cose_es256_public_point(key).is_some()
}

fn parse_assertion(assertion: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    if assertion.len() < HEADER_BYTES || assertion[..4] != *b"SYWB" || assertion[4] != 1 {
        return None
    }
    if assertion[5..8] != [0; 3] {
        return None
    }
    let authenticator_length = u16::from_le_bytes([assertion[8], assertion[9]]) as usize;
    let client_data_length = u16::from_le_bytes([assertion[10], assertion[11]]) as usize;
    if authenticator_length < AUTHENTICATOR_DATA_MIN
        || authenticator_length > MAX_AUTHENTICATOR_DATA
        || client_data_length == 0
        || client_data_length > MAX_CLIENT_DATA
    {
        return None
    }
    let signature_start = HEADER_BYTES + authenticator_length + client_data_length;
    if signature_start >= assertion.len() {
        return None
    }
    let signature_length = assertion.len() - signature_start;
    if signature_length == 0 || signature_length > MAX_SIGNATURE {
        return None
    }
    Some((
        &assertion[HEADER_BYTES..HEADER_BYTES + authenticator_length],
        &assertion[HEADER_BYTES + authenticator_length..signature_start],
        &assertion[signature_start..],
    ))
}

fn verify_authenticator_data(authenticator_data: &[u8]) -> bool {
    authenticator_data.len() >= AUTHENTICATOR_DATA_MIN
        && authenticator_data[..32] == sha256(RP_ID.as_bytes())
        && (authenticator_data[32] & (FLAG_USER_PRESENT | FLAG_USER_VERIFIED))
            == (FLAG_USER_PRESENT | FLAG_USER_VERIFIED)
}

fn verify_client_data(client_data: &[u8], challenge: &[u8; 32]) -> bool {
    if client_data.len() > MAX_CLIENT_DATA {
        return false
    }
    let mut kind = [0; 32];
    let mut encoded_challenge = [0; 64];
    let mut origin = [0; 64];
    let Some(kind_length) = json_string_field(client_data, b"type", &mut kind) else {
        return false
    };
    let Some(challenge_length) = json_string_field(
        client_data,
        b"challenge",
        &mut encoded_challenge,
    ) else {
        return false
    };
    let Some(origin_length) = json_string_field(client_data, b"origin", &mut origin) else {
        return false
    };
    let mut expected_challenge = [0; 64];
    let expected_length = base64url_encode(challenge, &mut expected_challenge);
    kind_length == b"webauthn.get".len()
        && kind[..kind_length] == *b"webauthn.get"
        && challenge_length == expected_length
        && encoded_challenge[..challenge_length] == expected_challenge[..expected_length]
        && valid_local_origin(&origin[..origin_length])
}

fn valid_local_origin(origin: &[u8]) -> bool {
    let prefix = ORIGIN.as_bytes();
    if origin == prefix {
        return true
    }
    if !origin.starts_with(prefix)
        || origin.get(prefix.len()) != Some(&b':')
        || origin.len() > prefix.len() + 6
    {
        return false
    }
    let port = &origin[prefix.len() + 1..];
    if port.is_empty() || port.first() == Some(&b'0') {
        return false
    }
    let mut value = 0u32;
    for byte in port {
        if !byte.is_ascii_digit() {
            return false
        }
        value = value * 10 + u32::from(*byte - b'0');
    }
    value <= u16::MAX as u32
}

fn json_string_field(input: &[u8], wanted: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut cursor = 0;
    skip_json_space(input, &mut cursor);
    if input.get(cursor) != Some(&b'{') {
        return None
    }
    cursor += 1;
    let mut found = None;
    loop {
        skip_json_space(input, &mut cursor);
        if input.get(cursor) == Some(&b'}') {
            cursor += 1;
            skip_json_space(input, &mut cursor);
            return (cursor == input.len()).then_some(found).flatten()
        }
        let mut key = [0; 32];
        let key_length = parse_json_string(input, &mut cursor, &mut key)?;
        skip_json_space(input, &mut cursor);
        if input.get(cursor) != Some(&b':') {
            return None
        }
        cursor += 1;
        skip_json_space(input, &mut cursor);
        if key_length == wanted.len() && key[..key_length] == *wanted {
            if found.is_some() {
                return None
            }
            let value_length = parse_json_string(input, &mut cursor, output)?;
            found = Some(value_length);
        } else {
            skip_json_value(input, &mut cursor)?;
        }
        skip_json_space(input, &mut cursor);
        match input.get(cursor) {
            Some(b',') => {
                cursor += 1;
                continue
            }
            Some(b'}') => {
                cursor += 1;
                skip_json_space(input, &mut cursor);
                return (cursor == input.len()).then_some(found).flatten()
            }
            _ => return None,
        }
    }
}

fn skip_json_space(input: &[u8], cursor: &mut usize) {
    while input.get(*cursor).is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t')) {
        *cursor += 1
    }
}

fn parse_json_string(input: &[u8], cursor: &mut usize, output: &mut [u8]) -> Option<usize> {
    if input.get(*cursor) != Some(&b'"') {
        return None
    }
    *cursor += 1;
    let mut length = 0;
    loop {
        let byte = *input.get(*cursor)?;
        *cursor += 1;
        match byte {
            b'"' => return Some(length),
            b'\\' => {
                let escaped = *input.get(*cursor)?;
                *cursor += 1;
                let value = match escaped {
                    b'"' | b'\\' | b'/' => escaped,
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    _ => return None,
                };
                if length >= output.len() {
                    return None
                }
                output[length] = value;
                length += 1;
            }
            byte if byte >= 0x20 => {
                if length >= output.len() {
                    return None
                }
                output[length] = byte;
                length += 1;
            }
            _ => return None,
        }
    }
}

fn skip_json_value(input: &[u8], cursor: &mut usize) -> Option<()> {
    match input.get(*cursor)? {
        b'"' => skip_json_string(input, cursor),
        b'{' | b'[' => {
            let opening = input[*cursor];
            let closing = if opening == b'{' { b'}' } else { b']' };
            let mut depth = 0;
            let mut in_string = false;
            let mut escaped = false;
            while let Some(byte) = input.get(*cursor) {
                *cursor += 1;
                if in_string {
                    if escaped {
                        escaped = false
                    } else if *byte == b'\\' {
                        escaped = true
                    } else if *byte == b'"' {
                        in_string = false
                    }
                } else if *byte == b'"' {
                    in_string = true
                } else if *byte == opening {
                    depth += 1
                } else if *byte == closing {
                    depth -= 1;
                    if depth == 0 {
                        return Some(())
                    }
                }
            }
            None
        }
        _ => {
            while input.get(*cursor).is_some_and(|byte| !matches!(byte, b',' | b'}')) {
                *cursor += 1
            }
            Some(())
        }
    }
}

fn skip_json_string(input: &[u8], cursor: &mut usize) -> Option<()> {
    if input.get(*cursor) != Some(&b'"') {
        return None
    }
    *cursor += 1;
    let mut escaped = false;
    while let Some(byte) = input.get(*cursor) {
        *cursor += 1;
        if escaped {
            escaped = false
        } else if *byte == b'\\' {
            escaped = true
        } else if *byte == b'"' {
            return Some(())
        } else if *byte < 0x20 {
            return None
        }
    }
    None
}

fn base64url_encode(input: &[u8], output: &mut [u8]) -> usize {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut cursor = 0;
    let mut index = 0;
    while index + 3 <= input.len() {
        let value = (u32::from(input[index]) << 16)
            | (u32::from(input[index + 1]) << 8)
            | u32::from(input[index + 2]);
        output[cursor] = ALPHABET[(value >> 18) as usize];
        output[cursor + 1] = ALPHABET[((value >> 12) & 63) as usize];
        output[cursor + 2] = ALPHABET[((value >> 6) & 63) as usize];
        output[cursor + 3] = ALPHABET[(value & 63) as usize];
        cursor += 4;
        index += 3;
    }
    let remaining = input.len() - index;
    if remaining != 0 {
        let value = u32::from(input[index]) << 16
            | if remaining == 2 { u32::from(input[index + 1]) << 8 } else { 0 };
        output[cursor] = ALPHABET[(value >> 18) as usize];
        output[cursor + 1] = ALPHABET[((value >> 12) & 63) as usize];
        cursor += 2;
        if remaining == 2 {
            output[cursor] = ALPHABET[((value >> 6) & 63) as usize];
            cursor += 1;
        }
    }
    cursor
}

fn verify_cose_key(key: &[u8], digest: &[u8; 32], signature: &[u8]) -> bool {
    let Some(public) = cose_es256_public_point(key) else { return false };
    let Some((r, s)) = parse_signature(signature) else {
        return false
    };
    if r == ZERO || s == ZERO || ge(r, ORDER) || ge(s, ORDER) {
        return false
    }
    let mut message = *digest;
    let e = from_be(&message);
    let w = inverse_mod(s, ORDER);
    let u1 = mul_mod(e, w, ORDER);
    let u2 = mul_mod(r, w, ORDER);
    let point = point_add(scalar_mul(Point::generator(), u1), scalar_mul(public, u2));
    let Some((x, _)) = affine(point) else {
        return false
    };
    let x = if ge(x, ORDER) { sub_raw(x, ORDER) } else { x };
    message.fill(0);
    x == r
}

fn cose_es256_public_point(key: &[u8]) -> Option<Point> {
    let (x, y, algorithm, curve, key_type) = parse_cose_key(key)?;
    if algorithm != -7
        || curve != 1
        || key_type != 2
        || ge(x, FIELD_MODULUS)
        || ge(y, FIELD_MODULUS)
    {
        return None
    }
    let public = Point { x, y, z: ONE };
    on_curve(public).then_some(public)
}

fn parse_signature(signature: &[u8]) -> Option<(U256, U256)> {
    if signature.len() == 64 {
        return Some((from_be(&signature[..32]), from_be(&signature[32..])))
    }
    if signature.first() != Some(&0x30) {
        return None
    }
    let mut cursor = 1;
    let sequence_length = der_length(signature, &mut cursor)?;
    if sequence_length != signature.len() - cursor {
        return None
    }
    let r = der_integer(signature, &mut cursor)?;
    let s = der_integer(signature, &mut cursor)?;
    (cursor == signature.len()).then_some((r, s))
}

fn der_length(input: &[u8], cursor: &mut usize) -> Option<usize> {
    let byte = *input.get(*cursor)?;
    *cursor += 1;
    if byte & 0x80 == 0 {
        return Some(byte as usize)
    }
    let count = (byte & 0x7f) as usize;
    if count == 0 || count > 2 || *cursor + count > input.len() {
        return None
    }
    let mut length = 0;
    for _ in 0..count {
        length = (length << 8) | usize::from(input[*cursor]);
        *cursor += 1;
    }
    Some(length)
}

fn der_integer(input: &[u8], cursor: &mut usize) -> Option<U256> {
    if input.get(*cursor) != Some(&2) {
        return None
    }
    *cursor += 1;
    let length = der_length(input, cursor)?;
    if length == 0 || length > 33 || *cursor + length > input.len() {
        return None
    }
    let bytes = &input[*cursor..*cursor + length];
    *cursor += length;
    if bytes[0] & 0x80 != 0 || (length > 1 && bytes[0] == 0 && bytes[1] & 0x80 == 0) {
        return None
    }
    let bytes = if length == 33 {
        if bytes[0] != 0 { return None }
        &bytes[1..]
    } else {
        bytes
    };
    Some(from_be(bytes))
}

fn parse_cose_key(input: &[u8]) -> Option<(U256, U256, i64, i64, i64)> {
    let mut reader = CborReader { input, cursor: 0 };
    let count = reader.map_length()?;
    if count > 16 {
        return None
    }
    let mut x = None;
    let mut y = None;
    let mut algorithm = None;
    let mut curve = None;
    let mut key_type = None;
    for _ in 0..count {
        let key = reader.integer()?;
        match key {
            -2 => x = Some(reader.bytes_32()?),
            -3 => y = Some(reader.bytes_32()?),
            1 => key_type = Some(reader.integer()?),
            3 => algorithm = Some(reader.integer()?),
            -1 => curve = Some(reader.integer()?),
            _ => reader.skip()?,
        }
    }
    (reader.cursor == input.len()).then_some((x?, y?, algorithm?, curve?, key_type?))
}

struct CborReader<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl CborReader<'_> {
    fn head(&mut self) -> Option<(u8, u64)> {
        let byte = *self.input.get(self.cursor)?;
        self.cursor += 1;
        let major = byte >> 5;
        let additional = byte & 0x1f;
        let width = match additional {
            0..=23 => 0,
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => return None,
        };
        if self.cursor + width > self.input.len() {
            return None
        }
        let start = self.cursor;
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => u64::from(self.input[start]),
            25 => u64::from(u16::from_be_bytes([self.input[start], self.input[start + 1]])),
            26 => u64::from(u32::from_be_bytes([
                self.input[start],
                self.input[start + 1],
                self.input[start + 2],
                self.input[start + 3],
            ])),
            27 => u64::from_be_bytes([
                self.input[start],
                self.input[start + 1],
                self.input[start + 2],
                self.input[start + 3],
                self.input[start + 4],
                self.input[start + 5],
                self.input[start + 6],
                self.input[start + 7],
            ]),
            _ => unreachable!(),
        };
        self.cursor += width;
        Some((major, value))
    }

    fn map_length(&mut self) -> Option<usize> {
        let (major, length) = self.head()?;
        (major == 5).then_some(length as usize)
    }

    fn integer(&mut self) -> Option<i64> {
        let (major, value) = self.head()?;
        match major {
            0 if value <= i64::MAX as u64 => Some(value as i64),
            1 if value < i64::MAX as u64 => Some(-1 - value as i64),
            _ => None,
        }
    }

    fn bytes_32(&mut self) -> Option<U256> {
        let (major, length) = self.head()?;
        if major != 2 || length != 32 || self.cursor + 32 > self.input.len() {
            return None
        }
        let value = from_be(&self.input[self.cursor..self.cursor + 32]);
        self.cursor += 32;
        Some(value)
    }

    fn skip(&mut self) -> Option<()> {
        let (major, value) = self.head()?;
        match major {
            0 | 1 | 7 => Some(()),
            2 | 3 => {
                let length = value as usize;
                (self.cursor + length <= self.input.len()).then(|| {
                    self.cursor += length
                })
            }
            4 | 5 => {
                let items = if major == 5 { value.saturating_mul(2) } else { value };
                for _ in 0..items {
                    self.skip()?
                }
                Some(())
            }
            _ => None,
        }
    }
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1,
        0x923f_82a4, 0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3,
        0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786,
        0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
        0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147,
        0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13,
        0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
        0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
        0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a,
        0x5b9c_ca4f, 0x682e_6ff3, 0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208,
        0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_length = (input.len() as u64).wrapping_mul(8);
    let blocks = input.len().saturating_add(9).div_ceil(64);
    for block in 0..blocks {
        let mut words: [u32; 64] = [0; 64];
        for index in 0..16 {
            let start = block * 64 + index * 4;
            let mut bytes = [0; 4];
            for (offset, byte) in bytes.iter_mut().enumerate() {
                *byte = if start + offset < input.len() {
                    input[start + offset]
                } else if start + offset == input.len() {
                    0x80
                } else if start + offset >= blocks * 64 - 8 {
                    bit_length.to_be_bytes()[start + offset - (blocks * 64 - 8)]
                } else {
                    0
                };
            }
            words[index] = u32::from_be_bytes(bytes)
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1)
        }
        let mut working = state;
        for index in 0..64 {
            let s1 = working[4].rotate_right(6)
                ^ working[4].rotate_right(11)
                ^ working[4].rotate_right(25);
            let choice = (working[4] & working[5]) ^ ((!working[4]) & working[6]);
            let temp1 = working[7]
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = working[0].rotate_right(2)
                ^ working[0].rotate_right(13)
                ^ working[0].rotate_right(22);
            let majority = (working[0] & working[1])
                ^ (working[0] & working[2])
                ^ (working[1] & working[2]);
            let temp2 = s0.wrapping_add(majority);
            working.copy_within(0..7, 1);
            working[0] = temp1.wrapping_add(temp2);
            working[4] = working[4].wrapping_add(temp1)
        }
        for index in 0..8 {
            state[index] = state[index].wrapping_add(working[index])
        }
    }
    let mut output = [0; 32];
    for (index, word) in state.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes())
    }
    output
}

fn from_be(bytes: &[u8]) -> U256 {
    let mut output = [0; 4];
    let start = bytes.len().saturating_sub(32);
    for (index, chunk) in bytes[start..].rchunks(8).enumerate() {
        let mut word = [0; 8];
        word[8 - chunk.len()..].copy_from_slice(chunk);
        output[index] = u64::from_be_bytes(word)
    }
    output
}

fn ge(left: U256, right: U256) -> bool {
    cmp(left, right) != Ordering::Less
}

fn cmp(left: U256, right: U256) -> Ordering {
    for index in (0..4).rev() {
        match left[index].cmp(&right[index]) {
            Ordering::Equal => {}
            result => return result,
        }
    }
    Ordering::Equal
}

fn add_raw(left: U256, right: U256) -> (U256, bool) {
    let mut output = [0; 4];
    let mut carry = false;
    for index in 0..4 {
        let (sum, first) = left[index].overflowing_add(right[index]);
        let (sum, second) = sum.overflowing_add(carry as u64);
        output[index] = sum;
        carry = first || second
    }
    (output, carry)
}

fn sub_raw(left: U256, right: U256) -> U256 {
    let mut output = [0; 4];
    let mut borrow = false;
    for index in 0..4 {
        let (difference, first) = left[index].overflowing_sub(right[index]);
        let (difference, second) = difference.overflowing_sub(borrow as u64);
        output[index] = difference;
        borrow = first || second
    }
    output
}

fn add_mod(left: U256, right: U256, modulus: U256) -> U256 {
    let (sum, carry) = add_raw(left, right);
    if carry || ge(sum, modulus) {
        sub_raw(sum, modulus)
    } else {
        sum
    }
}

fn sub_mod(left: U256, right: U256, modulus: U256) -> U256 {
    if ge(left, right) {
        sub_raw(left, right)
    } else {
        sub_raw(modulus, sub_raw(right, left))
    }
}

fn mul_mod(left: U256, right: U256, modulus: U256) -> U256 {
    let mut result = ZERO;
    let mut addend = left;
    for index in 0..256 {
        if right[index / 64] & (1 << (index % 64)) != 0 {
            result = add_mod(result, addend, modulus)
        }
        addend = add_mod(addend, addend, modulus)
    }
    result
}

fn inverse_mod(value: U256, modulus: U256) -> U256 {
    let exponent = sub_small(modulus, 2);
    let mut result = ONE;
    let mut base = value;
    for index in 0..256 {
        if exponent[index / 64] & (1 << (index % 64)) != 0 {
            result = mul_mod(result, base, modulus)
        }
        base = mul_mod(base, base, modulus)
    }
    result
}

fn sub_small(value: U256, amount: u64) -> U256 {
    let mut result = value;
    let (word, borrow) = result[0].overflowing_sub(amount);
    result[0] = word;
    if borrow {
        for slot in result.iter_mut().skip(1) {
            let (word, borrow) = slot.overflowing_sub(1);
            *slot = word;
            if !borrow {
                break
            }
        }
    }
    result
}

fn point_double(point: Point) -> Point {
    if point.is_infinity() || point.y == ZERO {
        return Point::infinity()
    }
    let yy = mul_mod(point.y, point.y, FIELD_MODULUS);
    let yyyy = mul_mod(yy, yy, FIELD_MODULUS);
    let zz = mul_mod(point.z, point.z, FIELD_MODULUS);
    let m = mul_mod(
        [
            3, 0, 0, 0,
        ],
        mul_mod(
            sub_mod(point.x, zz, FIELD_MODULUS),
            add_mod(point.x, zz, FIELD_MODULUS),
            FIELD_MODULUS,
        ),
        FIELD_MODULUS,
    );
    let s = mul_mod(
        [4, 0, 0, 0],
        mul_mod(point.x, yy, FIELD_MODULUS),
        FIELD_MODULUS,
    );
    let x = sub_mod(
        mul_mod(m, m, FIELD_MODULUS),
        add_mod(s, s, FIELD_MODULUS),
        FIELD_MODULUS,
    );
    let y = sub_mod(
        mul_mod(m, sub_mod(s, x, FIELD_MODULUS), FIELD_MODULUS),
        mul_mod([8, 0, 0, 0], yyyy, FIELD_MODULUS),
        FIELD_MODULUS,
    );
    let z = mul_mod([2, 0, 0, 0], mul_mod(point.y, point.z, FIELD_MODULUS), FIELD_MODULUS);
    Point { x, y, z }
}

fn point_add(left: Point, right: Point) -> Point {
    if left.is_infinity() {
        return right
    }
    if right.is_infinity() {
        return left
    }
    let z1z1 = mul_mod(left.z, left.z, FIELD_MODULUS);
    let z2z2 = mul_mod(right.z, right.z, FIELD_MODULUS);
    let u1 = mul_mod(left.x, z2z2, FIELD_MODULUS);
    let u2 = mul_mod(right.x, z1z1, FIELD_MODULUS);
    let s1 = mul_mod(left.y, mul_mod(right.z, z2z2, FIELD_MODULUS), FIELD_MODULUS);
    let s2 = mul_mod(right.y, mul_mod(left.z, z1z1, FIELD_MODULUS), FIELD_MODULUS);
    let h = sub_mod(u2, u1, FIELD_MODULUS);
    let r = sub_mod(s2, s1, FIELD_MODULUS);
    if h == ZERO {
        return if r == ZERO { point_double(left) } else { Point::infinity() }
    }
    let hh = mul_mod(h, h, FIELD_MODULUS);
    let hhh = mul_mod(h, hh, FIELD_MODULUS);
    let v = mul_mod(u1, hh, FIELD_MODULUS);
    let x = sub_mod(
        sub_mod(mul_mod(r, r, FIELD_MODULUS), hhh, FIELD_MODULUS),
        add_mod(v, v, FIELD_MODULUS),
        FIELD_MODULUS,
    );
    let y = sub_mod(
        mul_mod(r, sub_mod(v, x, FIELD_MODULUS), FIELD_MODULUS),
        mul_mod(s1, hhh, FIELD_MODULUS),
        FIELD_MODULUS,
    );
    let z = mul_mod(h, mul_mod(left.z, right.z, FIELD_MODULUS), FIELD_MODULUS);
    Point { x, y, z }
}

fn scalar_mul(point: Point, scalar: U256) -> Point {
    let mut result = Point::infinity();
    let mut current = point;
    for index in 0..256 {
        if scalar[index / 64] & (1 << (index % 64)) != 0 {
            result = point_add(result, current)
        }
        current = point_double(current)
    }
    result
}

fn affine(point: Point) -> Option<(U256, U256)> {
    if point.is_infinity() {
        return None
    }
    let inverse = inverse_mod(point.z, FIELD_MODULUS);
    let inverse_squared = mul_mod(inverse, inverse, FIELD_MODULUS);
    let x = mul_mod(point.x, inverse_squared, FIELD_MODULUS);
    let y = mul_mod(point.y, mul_mod(inverse_squared, inverse, FIELD_MODULUS), FIELD_MODULUS);
    Some((x, y))
}

fn on_curve(point: Point) -> bool {
    let left = mul_mod(point.y, point.y, FIELD_MODULUS);
    let x_squared = mul_mod(point.x, point.x, FIELD_MODULUS);
    let right = add_mod(
        sub_mod(
            mul_mod(point.x, x_squared, FIELD_MODULUS),
            mul_mod([3, 0, 0, 0], point.x, FIELD_MODULUS),
            FIELD_MODULUS,
        ),
        CURVE_B,
        FIELD_MODULUS,
    );
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATOR_KEY_HEX: &str = "a50102032620012158206b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2962258204fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
    const BROWSER_KEY_HEX: &str = "a50102032620012158205c4b9868b5f144608bf9eca9ce5c5ceb790ff7af5c9cc1be2a81c7df6eb65233225820237a2f5bdaeae3cacd36bf793ecc7667b9ca06cb2ec0bac5f5734a1d93588b52";
    const DIGEST_HEX: &str = "a79114a8a4a33276e206962ab3dec3a468008e9290c10693475d3fcdead2f35e";
    const DER_SIGNATURE_HEX: &str = "304402201586614b8713fdc2b38c4f3065ccc5556c7656edb950df82045fd49d8c5f47020220425bdf5781f923fb72631f21e38babe28178974036274e255079f786c6df225c";

    fn decode_into<const SIZE: usize>(hex: &str) -> [u8; SIZE] {
        let mut output = [0; SIZE];
        assert_eq!(hex.len(), SIZE * 2);
        for (index, slot) in output.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap()
        }
        output
    }

    #[test]
    fn genuine_browser_cose_public_keys_validate() {
        assert!(valid_cose_es256_public_key(&decode_into::<77>(GENERATOR_KEY_HEX)));
        assert!(valid_cose_es256_public_key(&decode_into::<77>(BROWSER_KEY_HEX)));
    }

    #[test]
    fn tampered_or_malformed_cose_public_keys_fail_validation() {
        let mut key = decode_into::<77>(BROWSER_KEY_HEX);
        key[10] ^= 1;
        assert!(!valid_cose_es256_public_key(&key));
        let mut key = decode_into::<77>(BROWSER_KEY_HEX);
        key[76] ^= 1;
        assert!(!valid_cose_es256_public_key(&key));
        let key = decode_into::<77>(BROWSER_KEY_HEX);
        assert!(!valid_cose_es256_public_key(&key[..76]));
        let mut padded = [0; 78];
        padded[..77].copy_from_slice(&key);
        padded[77] = 0;
        assert!(!valid_cose_es256_public_key(&padded));
        let mut algorithm = key;
        algorithm[4] = 0x27;
        assert!(!valid_cose_es256_public_key(&algorithm));
        let mut curve = key;
        curve[6] = 0x02;
        assert!(!valid_cose_es256_public_key(&curve));
        assert!(!valid_cose_es256_public_key(&[]));
    }

    #[test]
    fn sha256_matches_known_answers() {
        assert_eq!(
            sha256(b""),
            decode_into::<32>("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            sha256(b"abc"),
            decode_into::<32>("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn ecdsa_assertions_verify_openssl_vectors() {
        let key = decode_into::<77>(BROWSER_KEY_HEX);
        let digest = decode_into::<32>(DIGEST_HEX);
        let signature = decode_into::<70>(DER_SIGNATURE_HEX);
        assert!(verify_cose_key(&key, &digest, &signature));
        let mut corrupted_digest = digest;
        corrupted_digest[0] ^= 1;
        assert!(!verify_cose_key(&key, &corrupted_digest, &signature));
        let mut corrupted_signature = signature;
        corrupted_signature[69] ^= 1;
        assert!(!verify_cose_key(&key, &digest, &corrupted_signature));
        let mut raw_signature = [0; 64];
        raw_signature[..32].copy_from_slice(&signature[4..36]);
        raw_signature[32..].copy_from_slice(&signature[38..70]);
        assert!(verify_cose_key(&key, &digest, &raw_signature));
    }
}
