use core::str;

pub const DEFAULT_REQUEST_HEADERS: usize = 32;
pub const DEFAULT_RESPONSE_HEADERS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
        }
    }

    fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "GET" => Ok(Self::Get),
            "HEAD" => Ok(Self::Head),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "PATCH" => Ok(Self::Patch),
            "DELETE" => Ok(Self::Delete),
            "OPTIONS" => Ok(Self::Options),
            _ => Err(ParseError::UnsupportedMethod),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Version {
    Http10,
    Http11,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

impl<'a> Header<'a> {
    pub const fn new(name: &'a str, value: &'a str) -> Self {
        Self { name, value }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request<'a, const HEADERS: usize = DEFAULT_REQUEST_HEADERS> {
    pub method: Method,
    pub path: &'a str,
    pub version: Version,
    pub body: &'a [u8],
    headers: [Option<Header<'a>>; HEADERS],
    header_count: usize,
}

impl<'a, const HEADERS: usize> Request<'a, HEADERS> {
    pub fn headers(&self) -> impl Iterator<Item = Header<'a>> + '_ {
        self.headers[..self.header_count].iter().flatten().copied()
    }

    pub fn header(&self, name: &str) -> Option<&'a str> {
        self.headers()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value)
    }

    pub fn query(&self) -> Option<&'a str> {
        self.path.split_once('?').map(|(_, query)| query)
    }

    pub fn path_without_query(&self) -> &'a str {
        self.path
            .split_once('?')
            .map_or(self.path, |(path, _)| path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedRequest<'a, const HEADERS: usize = DEFAULT_REQUEST_HEADERS> {
    pub request: Request<'a, HEADERS>,
    pub consumed: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    HeaderCapacity,
    HeadersTooLarge,
    Incomplete,
    InvalidContentLength,
    InvalidHeader,
    InvalidPath,
    InvalidRequestLine,
    InvalidUtf8,
    Protocol(ghostos_protocol::ProtocolError),
    UnsupportedMethod,
    UnsupportedTransferEncoding,
    UnsupportedVersion,
}

pub fn parse_request_checked<'a, const HEADERS: usize>(
    guard: &mut ghostos_protocol::ProtocolGuard,
    sequence: u64,
    bytes: &'a [u8],
) -> Result<ParsedRequest<'a, HEADERS>, ParseError> {
    guard
        .require_class(ghostos_protocol::TrafficClass::Http)
        .map_err(ParseError::Protocol)?;
    guard
        .validate_message(bytes.len())
        .map_err(ParseError::Protocol)?;
    let parsed = parse_request(bytes)?;
    guard
        .accept_sequence(sequence)
        .map_err(ParseError::Protocol)?;
    Ok(parsed)
}

pub fn parse_request<const HEADERS: usize>(
    bytes: &[u8],
) -> Result<ParsedRequest<'_, HEADERS>, ParseError> {
    let header_end = find_bytes(bytes, b"\r\n\r\n").ok_or(ParseError::Incomplete)?;
    let head = str::from_utf8(&bytes[..header_end]).map_err(|_| ParseError::InvalidUtf8)?;
    let (request_line, header_lines) = head.split_once("\r\n").map_or((head, ""), |parts| parts);
    let mut parts = request_line.split(' ');
    let method = Method::parse(parts.next().ok_or(ParseError::InvalidRequestLine)?)?;
    let path = parts.next().ok_or(ParseError::InvalidRequestLine)?;
    let version = match parts.next().ok_or(ParseError::InvalidRequestLine)? {
        "HTTP/1.0" => Version::Http10,
        "HTTP/1.1" => Version::Http11,
        _ => return Err(ParseError::UnsupportedVersion),
    };
    if parts.next().is_some() {
        return Err(ParseError::InvalidRequestLine);
    }
    if !path.starts_with('/') || path.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
        return Err(ParseError::InvalidPath);
    }

    let mut headers = [None; HEADERS];
    let mut header_count = 0;
    let mut content_length = 0;
    let mut has_content_length = false;
    let mut has_transfer_encoding = false;
    for line in header_lines.split("\r\n").filter(|line| !line.is_empty()) {
        let (name, raw_value) = line.split_once(':').ok_or(ParseError::InvalidHeader)?;
        if name.is_empty() || !name.bytes().all(is_header_name_byte) {
            return Err(ParseError::InvalidHeader);
        }
        let value = raw_value.trim_matches([' ', '\t']);
        if value
            .bytes()
            .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
        {
            return Err(ParseError::InvalidHeader);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if has_content_length || has_transfer_encoding {
                return Err(ParseError::InvalidContentLength);
            }
            content_length = parse_decimal(value)?;
            has_content_length = true;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if has_transfer_encoding || has_content_length {
                return Err(ParseError::UnsupportedTransferEncoding);
            }
            has_transfer_encoding = true;
            if !value.eq_ignore_ascii_case("identity") {
                return Err(ParseError::UnsupportedTransferEncoding);
            }
        }
        let slot = headers
            .get_mut(header_count)
            .ok_or(ParseError::HeaderCapacity)?;
        *slot = Some(Header { name, value });
        header_count += 1;
    }

    let body_start = header_end
        .checked_add(4)
        .ok_or(ParseError::HeadersTooLarge)?;
    let consumed = body_start
        .checked_add(content_length)
        .ok_or(ParseError::InvalidContentLength)?;
    let body = bytes
        .get(body_start..consumed)
        .ok_or(ParseError::Incomplete)?;
    Ok(ParsedRequest {
        request: Request {
            method,
            path,
            version,
            body,
            headers,
            header_count,
        },
        consumed,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct StatusCode(u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const NO_CONTENT: Self = Self(204);
    pub const BAD_REQUEST: Self = Self(400);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const PAYLOAD_TOO_LARGE: Self = Self(413);
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);

    pub const fn new(value: u16) -> Option<Self> {
        if value >= 100 && value <= 599 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }

    const fn reason(self) -> &'static str {
        match self.0 {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            500 => "Internal Server Error",
            503 => "Service Unavailable",
            _ => "Response",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Response<'a, const HEADERS: usize = DEFAULT_RESPONSE_HEADERS> {
    pub status: StatusCode,
    pub body: &'a [u8],
    headers: [Option<Header<'a>>; HEADERS],
    header_count: usize,
}

impl<'a, const HEADERS: usize> Response<'a, HEADERS> {
    pub const fn new(status: StatusCode, body: &'a [u8]) -> Self {
        Self {
            status,
            body,
            headers: [None; HEADERS],
            header_count: 0,
        }
    }

    pub fn with_header(mut self, name: &'a str, value: &'a str) -> Result<Self, EncodeError> {
        if name.is_empty() || !name.bytes().all(is_header_name_byte) || contains_newline(value) {
            return Err(EncodeError::InvalidHeader);
        }
        let slot = self
            .headers
            .get_mut(self.header_count)
            .ok_or(EncodeError::HeaderCapacity)?;
        *slot = Some(Header { name, value });
        self.header_count += 1;
        Ok(self)
    }

    pub fn headers(&self) -> impl Iterator<Item = Header<'a>> + '_ {
        self.headers[..self.header_count].iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall { required: usize },
    HeaderCapacity,
    InvalidHeader,
}

pub fn encode_response<const HEADERS: usize>(
    response: Response<'_, HEADERS>,
    destination: &mut [u8],
) -> Result<usize, EncodeError> {
    let mut writer = Writer::new(destination);
    writer.push(b"HTTP/1.1 ")?;
    writer.decimal(response.status.as_u16() as usize)?;
    writer.byte(b' ')?;
    writer.push(response.status.reason().as_bytes())?;
    writer.push(b"\r\ncontent-length: ")?;
    writer.decimal(response.body.len())?;
    writer.push(b"\r\nconnection: close\r\n")?;
    for header in response.headers() {
        if header.name.eq_ignore_ascii_case("content-length")
            || header.name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        writer.push(header.name.as_bytes())?;
        writer.push(b": ")?;
        writer.push(header.value.as_bytes())?;
        writer.push(b"\r\n")?;
    }
    writer.push(b"\r\n")?;
    writer.push(response.body)?;
    Ok(writer.len)
}

fn parse_decimal(value: &str) -> Result<usize, ParseError> {
    if value.is_empty() {
        return Err(ParseError::InvalidContentLength);
    }
    value.bytes().try_fold(0usize, |current, byte| {
        if !byte.is_ascii_digit() {
            return Err(ParseError::InvalidContentLength);
        }
        current
            .checked_mul(10)
            .and_then(|number| number.checked_add((byte - b'0') as usize))
            .ok_or(ParseError::InvalidContentLength)
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

const fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn contains_newline(value: &str) -> bool {
    value.bytes().any(|byte| byte == b'\r' || byte == b'\n')
}

struct Writer<'a> {
    destination: &'a mut [u8],
    len: usize,
}

impl<'a> Writer<'a> {
    const fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            len: 0,
        }
    }

    fn byte(&mut self, byte: u8) -> Result<(), EncodeError> {
        self.push(&[byte])
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(EncodeError::BufferTooSmall {
                required: usize::MAX,
            })?;
        let target = self
            .destination
            .get_mut(self.len..end)
            .ok_or(EncodeError::BufferTooSmall { required: end })?;
        target.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn decimal(&mut self, mut value: usize) -> Result<(), EncodeError> {
        let mut digits = [0; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                return self.push(&digits[start..]);
            }
        }
    }
}
