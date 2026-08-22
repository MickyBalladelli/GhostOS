use core::{fmt, str};

use crate::Error;

pub const MAX_MODEL_NAME_BYTES: usize = 96;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ModelName {
    bytes: [u8; MAX_MODEL_NAME_BYTES],
    length: u8,
}

impl ModelName {
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.is_empty() || value.len() > MAX_MODEL_NAME_BYTES || value.as_bytes().contains(&0) {
            return Err(Error::InvalidRequest);
        }
        let mut bytes = [0; MAX_MODEL_NAME_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.bytes[..self.length as usize])
            .expect("model name contains source UTF-8")
    }
}

impl fmt::Debug for ModelName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ModelName")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionKind {
    Text,
    Chat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionRequest<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub max_tokens: u32,
    pub stream: bool,
    pub kind: CompletionKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAiRequest<'a> {
    ListModels,
    Complete(CompletionRequest<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionResponse<'a> {
    pub id: u64,
    pub model: &'a str,
    pub text: &'a str,
    pub kind: CompletionKind,
    pub created_at: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub finished: bool,
}

pub struct OpenAiCodec;

impl OpenAiCodec {
    pub fn decode<'a>(path: &str, body: &'a [u8]) -> Result<OpenAiRequest<'a>, Error> {
        match path {
            "/v1/models" if body.is_empty() => Ok(OpenAiRequest::ListModels),
            "/v1/completions" => Self::decode_completion(body, CompletionKind::Text),
            "/v1/chat/completions" => Self::decode_completion(body, CompletionKind::Chat),
            _ => Err(Error::UnsupportedEndpoint),
        }
    }

    pub fn encode_completion(
        response: CompletionResponse<'_>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        let mut writer = JsonWriter::new(destination);
        writer.push(match response.kind {
            CompletionKind::Text => b"{\"id\":\"cmpl-",
            CompletionKind::Chat => b"{\"id\":\"chatcmpl-",
        })?;
        writer.decimal(response.id)?;
        writer.push(b"\",\"object\":\"")?;
        match (response.kind, response.finished) {
            (CompletionKind::Text, _) => writer.push(b"text_completion")?,
            (CompletionKind::Chat, true) => writer.push(b"chat.completion")?,
            (CompletionKind::Chat, false) => writer.push(b"chat.completion.chunk")?,
        }
        writer.push(b"\",\"created\":")?;
        writer.decimal(response.created_at)?;
        writer.push(b",\"model\":\"")?;
        writer.escaped(response.model.as_bytes())?;
        if response.finished {
            match response.kind {
                CompletionKind::Text => {
                    writer.push(b"\",\"choices\":[{\"index\":0,\"text\":\"")?;
                    writer.escaped(response.text.as_bytes())?;
                    writer
                        .push(b"\",\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":")?;
                }
                CompletionKind::Chat => {
                    writer.push(
                        b"\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"",
                    )?;
                    writer.escaped(response.text.as_bytes())?;
                    writer
                        .push(b"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":")?;
                }
            }
            writer.decimal(response.prompt_tokens as u64)?;
            writer.push(b",\"completion_tokens\":")?;
            writer.decimal(response.completion_tokens as u64)?;
            writer.push(b",\"total_tokens\":")?;
            writer.decimal(
                u64::from(response.prompt_tokens) + u64::from(response.completion_tokens),
            )?;
            writer.push(b"}}")
        } else {
            match response.kind {
                CompletionKind::Text => {
                    writer.push(b"\",\"choices\":[{\"index\":0,\"text\":\"")?;
                    writer.escaped(response.text.as_bytes())?;
                    writer.push(b"\",\"finish_reason\":null}]}")
                }
                CompletionKind::Chat => {
                    writer.push(b"\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"")?;
                    writer.escaped(response.text.as_bytes())?;
                    writer.push(b"\"},\"finish_reason\":null}]}")
                }
            }
        }?;
        Ok(writer.len())
    }

    pub fn encode_sse_chunk(
        response: CompletionResponse<'_>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        if response.finished {
            return copy_exact(b"data: [DONE]\n\n", destination);
        }
        if destination.len() < 8 {
            return Err(Error::BufferTooSmall { required: 8 });
        }
        destination[..6].copy_from_slice(b"data: ");
        let encoded = Self::encode_completion(response, &mut destination[6..])?;
        let required = 6 + encoded + 2;
        if destination.len() < required {
            return Err(Error::BufferTooSmall { required });
        }
        destination[6 + encoded..required].copy_from_slice(b"\n\n");
        Ok(required)
    }

    pub fn encode_model_list(
        models: impl Iterator<Item = ModelName>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        let mut writer = JsonWriter::new(destination);
        writer.push(b"{\"object\":\"list\",\"data\":[")?;
        for (index, model) in models.enumerate() {
            if index != 0 {
                writer.byte(b',')?
            }
            writer.push(b"{\"id\":\"")?;
            writer.escaped(model.as_str().as_bytes())?;
            writer.push(b"\",\"object\":\"model\",\"owned_by\":\"ghostos\"}")?;
        }
        writer.push(b"]}")?;
        Ok(writer.len())
    }

    pub fn encode_error(message: &str, destination: &mut [u8]) -> Result<usize, Error> {
        let mut writer = JsonWriter::new(destination);
        writer.push(b"{\"error\":{\"message\":\"")?;
        writer.escaped(message.as_bytes())?;
        writer.push(b"\",\"type\":\"invalid_request_error\"}}")?;
        Ok(writer.len())
    }

    fn decode_completion<'a>(
        body: &'a [u8],
        kind: CompletionKind,
    ) -> Result<OpenAiRequest<'a>, Error> {
        let body = str::from_utf8(body).map_err(|_| Error::InvalidRequest)?;
        let model = json_string(body, "model", false)?;
        let prompt = match kind {
            CompletionKind::Text => json_string(body, "prompt", false)?,
            CompletionKind::Chat => json_string(body, "content", true)?,
        };
        let max_tokens = json_u32(body, "max_completion_tokens")
            .or_else(|| json_u32(body, "max_tokens"))
            .unwrap_or(256);
        if max_tokens == 0 {
            return Err(Error::InvalidRequest);
        }
        let stream = json_bool(body, "stream").unwrap_or(false);
        Ok(OpenAiRequest::Complete(CompletionRequest {
            model,
            prompt,
            max_tokens,
            stream,
            kind,
        }))
    }
}

/// Five-byte gRPC record framing around a compact protobuf request.
///
/// Request fields are model (1), prompt (2), max_tokens (3), stream (4), and
/// completion kind (5). Response fields are id (1), model (2), text (3),
/// prompt_tokens (4), completion_tokens (5), and finished (6).
pub struct GrpcCodec;

impl GrpcCodec {
    pub fn decode(frame: &[u8]) -> Result<CompletionRequest<'_>, Error> {
        if frame.len() < 5 || frame[0] != 0 {
            return Err(Error::UnsupportedProtocol);
        }
        let length = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        if frame.len() != length + 5 {
            return Err(Error::InvalidRequest);
        }
        let message = &frame[5..];
        let model = protobuf_string(message, 1)?.ok_or(Error::InvalidRequest)?;
        let prompt = protobuf_string(message, 2)?.ok_or(Error::InvalidRequest)?;
        let max_tokens = protobuf_varint(message, 3)?
            .unwrap_or(256)
            .try_into()
            .map_err(|_| Error::InvalidRequest)?;
        if max_tokens == 0 {
            return Err(Error::InvalidRequest);
        }
        let stream = protobuf_varint(message, 4)?.unwrap_or(0) != 0;
        let kind = match protobuf_varint(message, 5)?.unwrap_or(1) {
            0 => CompletionKind::Text,
            1 => CompletionKind::Chat,
            _ => return Err(Error::InvalidRequest),
        };
        Ok(CompletionRequest {
            model,
            prompt,
            max_tokens,
            stream,
            kind,
        })
    }

    pub fn encode(
        response: CompletionResponse<'_>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        if destination.len() < 5 {
            return Err(Error::BufferTooSmall { required: 5 });
        }
        destination[0] = 0;
        let mut writer = ProtobufWriter::new(&mut destination[5..]);
        writer.varint_field(1, response.id)?;
        writer.bytes_field(2, response.model.as_bytes())?;
        writer.bytes_field(3, response.text.as_bytes())?;
        writer.varint_field(4, response.prompt_tokens as u64)?;
        writer.varint_field(5, response.completion_tokens as u64)?;
        writer.varint_field(6, response.finished as u64)?;
        let message_length = writer.len();
        destination[1..5].copy_from_slice(&(message_length as u32).to_be_bytes());
        Ok(message_length + 5)
    }
}

fn json_string<'a>(body: &'a str, key: &str, last: bool) -> Result<&'a str, Error> {
    let needle = key.as_bytes();
    let bytes = body.as_bytes();
    let mut found = None;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(relative) = bytes[cursor..]
            .windows(needle.len() + 2)
            .position(|window| {
                window.first() == Some(&b'"')
                    && window.last() == Some(&b'"')
                    && &window[1..window.len() - 1] == needle
            })
        else {
            break;
        };
        let key_start = cursor + relative;
        let mut value = key_start + needle.len() + 2;
        skip_space(bytes, &mut value);
        if bytes.get(value) != Some(&b':') {
            cursor = value.saturating_add(1);
            continue;
        }
        value += 1;
        skip_space(bytes, &mut value);
        if bytes.get(value) != Some(&b'"') {
            return Err(Error::InvalidRequest);
        }
        value += 1;
        let start = value;
        let mut escaped = false;
        while let Some(byte) = bytes.get(value).copied() {
            if byte == b'"' && !escaped {
                let candidate = &body[start..value];
                found = Some(candidate);
                cursor = value + 1;
                break;
            }
            if escaped {
                escaped = false
            } else if byte == b'\\' {
                escaped = true
            }
            value += 1;
        }
        if value >= bytes.len() {
            return Err(Error::InvalidRequest);
        }
        if !last {
            return found.ok_or(Error::InvalidRequest);
        }
    }
    found.ok_or(Error::InvalidRequest)
}

fn json_u32(body: &str, key: &str) -> Option<u32> {
    let bytes = body.as_bytes();
    let needle = key.as_bytes();
    let start = bytes.windows(needle.len() + 2).position(|window| {
        window.first() == Some(&b'"')
            && window.last() == Some(&b'"')
            && &window[1..window.len() - 1] == needle
    })?;
    let mut cursor = start + needle.len() + 2;
    skip_space(bytes, &mut cursor);
    if bytes.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    skip_space(bytes, &mut cursor);
    let number_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1
    }
    body[number_start..cursor].parse().ok()
}

fn json_bool(body: &str, key: &str) -> Option<bool> {
    let bytes = body.as_bytes();
    let needle = key.as_bytes();
    let start = bytes.windows(needle.len() + 2).position(|window| {
        window.first() == Some(&b'"')
            && window.last() == Some(&b'"')
            && &window[1..window.len() - 1] == needle
    })?;
    let mut cursor = start + needle.len() + 2;
    skip_space(bytes, &mut cursor);
    if bytes.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    skip_space(bytes, &mut cursor);
    if bytes[cursor..].starts_with(b"true") {
        Some(true)
    } else if bytes[cursor..].starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while bytes
        .get(*cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *cursor += 1
    }
}

fn protobuf_string(message: &[u8], field: u64) -> Result<Option<&str>, Error> {
    let Some(bytes) = protobuf_bytes(message, field)? else {
        return Ok(None);
    };
    str::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Error::InvalidRequest)
}

fn protobuf_bytes(message: &[u8], field: u64) -> Result<Option<&[u8]>, Error> {
    let mut cursor = 0;
    while cursor < message.len() {
        let key = read_varint(message, &mut cursor)?;
        let wire = key & 7;
        let current = key >> 3;
        match wire {
            0 => {
                let _ = read_varint(message, &mut cursor)?;
            }
            2 => {
                let length = read_varint(message, &mut cursor)? as usize;
                let end = cursor.checked_add(length).ok_or(Error::InvalidRequest)?;
                let value = message.get(cursor..end).ok_or(Error::InvalidRequest)?;
                cursor = end;
                if current == field {
                    return Ok(Some(value));
                }
            }
            _ => return Err(Error::UnsupportedProtocol),
        }
    }
    Ok(None)
}

fn protobuf_varint(message: &[u8], field: u64) -> Result<Option<u64>, Error> {
    let mut cursor = 0;
    while cursor < message.len() {
        let key = read_varint(message, &mut cursor)?;
        let wire = key & 7;
        let current = key >> 3;
        match wire {
            0 => {
                let value = read_varint(message, &mut cursor)?;
                if current == field {
                    return Ok(Some(value));
                }
            }
            2 => {
                let length = read_varint(message, &mut cursor)? as usize;
                cursor = cursor.checked_add(length).ok_or(Error::InvalidRequest)?;
                if cursor > message.len() {
                    return Err(Error::InvalidRequest);
                }
            }
            _ => return Err(Error::UnsupportedProtocol),
        }
    }
    Ok(None)
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*cursor).ok_or(Error::InvalidRequest)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return Err(Error::InvalidRequest);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::InvalidRequest)
}

struct JsonWriter<'a> {
    destination: &'a mut [u8],
    length: usize,
}

impl<'a> JsonWriter<'a> {
    fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            length: 0,
        }
    }

    fn len(&self) -> usize {
        self.length
    }

    fn byte(&mut self, byte: u8) -> Result<(), Error> {
        if self.length == self.destination.len() {
            return Err(Error::BufferTooSmall {
                required: self.length + 1,
            });
        }
        self.destination[self.length] = byte;
        self.length += 1;
        Ok(())
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let required = self
            .length
            .checked_add(bytes.len())
            .ok_or(Error::Capacity)?;
        if required > self.destination.len() {
            return Err(Error::BufferTooSmall { required });
        }
        self.destination[self.length..required].copy_from_slice(bytes);
        self.length = required;
        Ok(())
    }

    fn escaped(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for byte in bytes {
            match byte {
                b'"' => self.push(br#"\""#)?,
                b'\\' => self.push(br#"\\"#)?,
                b'\n' => self.push(br#"\n"#)?,
                b'\r' => self.push(br#"\r"#)?,
                b'\t' => self.push(br#"\t"#)?,
                0..=31 => return Err(Error::InvalidRequest),
                value => self.byte(*value)?,
            }
        }
        Ok(())
    }

    fn decimal(&mut self, value: u64) -> Result<(), Error> {
        let mut digits = [0; 20];
        let length = decimal_bytes(value, &mut digits);
        self.push(&digits[..length])
    }
}

struct ProtobufWriter<'a> {
    destination: &'a mut [u8],
    length: usize,
}

impl<'a> ProtobufWriter<'a> {
    fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            length: 0,
        }
    }

    fn len(&self) -> usize {
        self.length
    }

    fn byte(&mut self, byte: u8) -> Result<(), Error> {
        if self.length == self.destination.len() {
            return Err(Error::BufferTooSmall {
                required: self.length + 1,
            });
        }
        self.destination[self.length] = byte;
        self.length += 1;
        Ok(())
    }

    fn varint(&mut self, mut value: u64) -> Result<(), Error> {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80
            }
            self.byte(byte)?;
            if value == 0 {
                return Ok(());
            }
        }
    }

    fn varint_field(&mut self, field: u64, value: u64) -> Result<(), Error> {
        self.varint(field << 3)?;
        self.varint(value)
    }

    fn bytes_field(&mut self, field: u64, value: &[u8]) -> Result<(), Error> {
        self.varint(field << 3 | 2)?;
        self.varint(value.len() as u64)?;
        let required = self
            .length
            .checked_add(value.len())
            .ok_or(Error::Capacity)?;
        if required > self.destination.len() {
            return Err(Error::BufferTooSmall { required });
        }
        self.destination[self.length..required].copy_from_slice(value);
        self.length = required;
        Ok(())
    }
}

fn decimal_bytes(mut value: u64, destination: &mut [u8; 20]) -> usize {
    if value == 0 {
        destination[0] = b'0';
        return 1;
    }
    let mut reverse = [0; 20];
    let mut length = 0;
    while value != 0 {
        reverse[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
    }
    for index in 0..length {
        destination[index] = reverse[length - index - 1]
    }
    length
}

fn copy_exact(source: &[u8], destination: &mut [u8]) -> Result<usize, Error> {
    if destination.len() < source.len() {
        return Err(Error::BufferTooSmall {
            required: source.len(),
        });
    }
    destination[..source.len()].copy_from_slice(source);
    Ok(source.len())
}
