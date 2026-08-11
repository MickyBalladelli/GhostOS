use synos_status::{AuditContext, PublicError, RetryHint, Status};

use crate::{DEFAULT_RESPONSE_HEADERS, EncodeError, Response, StatusCode};

pub fn error_response<'a>(
    error: PublicError,
    destination: &'a mut [u8],
) -> Result<Response<'a, DEFAULT_RESPONSE_HEADERS>, EncodeError> {
    let length = encode_error_body(error, destination)?;
    Response::new(status_code(error.code), &destination[..length])
        .with_header("content-type", "application/json")
        .and_then(|response| response.with_header("cache-control", "no-store"))
}

pub fn encode_error_http_response(
    error: PublicError,
    destination: &mut [u8],
) -> Result<usize, EncodeError> {
    let mut body = [0; 512];
    let response = error_response(error, &mut body)?;
    crate::encode_response(response, destination)
}

pub fn encode_error_body(
    error: PublicError,
    destination: &mut [u8],
) -> Result<usize, EncodeError> {
    let mut writer = ErrorWriter::new(destination);
    writer.push(b"{\"code\":")?;
    writer.decimal(error.code.raw() as u128)?;
    writer.push(b",\"operation\":")?;
    writer.decimal(error.operation as u128)?;
    writer.push(b",\"message\":\"")?;
    writer.push(error.code.message().as_bytes())?;
    writer.push(b"\",\"action\":\"")?;
    writer.push(error.code.operator_action().as_bytes())?;
    writer.push(b"\",\"impact\":\"")?;
    writer.push(error.code.operator_impact().as_bytes())?;
    writer.push(b"\",\"retry\":\"")?;
    writer.push(error.retry.label().as_bytes())?;
    writer.push(b"\",\"retry_safety\":\"")?;
    writer.push(error.retry.safety().as_bytes())?;
    match error.retry {
        RetryHint::Never | RetryHint::Immediate => {}
        RetryHint::AfterUs(delay) => {
            writer.push(b"\",\"retry_after_us\":")?;
            writer.decimal(delay as u128)?;
        }
    }
    writer.push(b",\"audit\":{\"correlation\":\"")?;
    writer.hex(error.audit.correlation)?;
    writer.push(b"\",\"node\":")?;
    writer.decimal(error.audit.node as u128)?;
    writer.push(b"}}")?;
    Ok(writer.len())
}

pub fn status_code(status: Status) -> StatusCode {
    if status == Status::ACCESS_DENIED {
        StatusCode::FORBIDDEN
    } else if status == Status::NOT_FOUND {
        StatusCode::NOT_FOUND
    } else if status == Status::BUSY || status == Status::NO_SPACE {
        StatusCode::SERVICE_UNAVAILABLE
    } else if status == Status::INVALID_ARGUMENT
        || status == Status::ALREADY_EXISTS
    {
        StatusCode::BAD_REQUEST
    } else if status == Status::INVALID_PATH {
        StatusCode::BAD_REQUEST
    } else if status == Status::METHOD_NOT_ALLOWED {
        StatusCode::METHOD_NOT_ALLOWED
    } else if status == Status::REQUEST_TOO_LARGE {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub const fn route_error(
    error: crate::RouteError,
    audit: AuditContext,
) -> PublicError {
    let (code, retry) = match error {
        crate::RouteError::AccessDenied => (Status::ACCESS_DENIED, RetryHint::Never),
        crate::RouteError::Capacity => (Status::NO_SPACE, RetryHint::AfterUs(1_000_000)),
        crate::RouteError::Duplicate => (Status::ALREADY_EXISTS, RetryHint::Never),
        crate::RouteError::Handler => (Status::INTERNAL, RetryHint::AfterUs(1_000_000)),
        crate::RouteError::InvalidPath => (Status::INVALID_PATH, RetryHint::Never),
        crate::RouteError::MethodNotAllowed => (Status::METHOD_NOT_ALLOWED, RetryHint::Never),
        crate::RouteError::NotFound => (Status::NOT_FOUND, RetryHint::Never),
    };
    PublicError::new(code, synos_status::operation::HTTP_ROUTE, retry, audit)
}

pub const fn parse_error(error: crate::ParseError, audit: AuditContext) -> PublicError {
    let retry = match error {
        crate::ParseError::Incomplete => RetryHint::Immediate,
        _ => RetryHint::Never,
    };
    PublicError::new(
        Status::INVALID_ARGUMENT,
        synos_status::operation::HTTP_PARSE,
        retry,
        audit,
    )
}

struct ErrorWriter<'a> {
    destination: &'a mut [u8],
    length: usize,
}

impl<'a> ErrorWriter<'a> {
    const fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            length: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self
            .length
            .checked_add(bytes.len())
            .ok_or(EncodeError::BufferTooSmall { required: usize::MAX })?;
        let destination = self
            .destination
            .get_mut(self.length..end)
            .ok_or(EncodeError::BufferTooSmall { required: end })?;
        destination.copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }

    fn decimal(&mut self, mut value: u128) -> Result<(), EncodeError> {
        let mut digits = [0; 39];
        let mut count = 0;
        if value == 0 {
            return self.push(b"0");
        }
        while value != 0 {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
        }
        while count != 0 {
            count -= 1;
            self.push(&digits[count..count + 1])?;
        }
        Ok(())
    }

    fn hex(&mut self, mut value: u128) -> Result<(), EncodeError> {
        let mut digits = [b'0'; 32];
        for digit in digits.iter_mut().rev() {
            *digit = b"0123456789abcdef"[(value & 0xf) as usize];
            value >>= 4;
        }
        self.push(&digits)
    }

    const fn len(&self) -> usize {
        self.length
    }
}
