#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

pub mod audit;
pub mod diagnostics;
pub mod editor;
pub mod interpreter;
pub mod jobs;
pub mod parser;
pub mod render;

pub const MAX_LINE_BYTES: usize = 512;
pub const MAX_TOKEN_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyRunning,
    Capacity,
    CommandFailed(Status),
    DependencyFailed,
    InvalidHandle,
    InvalidSyntax,
    InvalidValue,
    JobNotFound,
    LineTooLong,
    MissingArgument,
    NoActiveCommand,
    NotOwner,
    QueueFull,
    TokenTooLong,
    TooManyArguments,
    TooManyStages,
    AmbiguousCommand,
    UnknownArgument,
    UnknownCommand,
    UnterminatedQuote,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::CommandFailed(status) => status,
            Self::Capacity | Self::LineTooLong | Self::QueueFull => Status::NO_SPACE,
            Self::JobNotFound | Self::UnknownCommand => Status::NOT_FOUND,
            Self::NotOwner => Status::ACCESS_DENIED,
            Self::AlreadyRunning => Status::BUSY,
            Self::DependencyFailed => {
                Status::new(Severity::Error, facility::SHELL, 1, 0)
                    .expect("valid shell status")
            }
            Self::InvalidHandle
            | Self::InvalidSyntax
            | Self::InvalidValue
            | Self::AmbiguousCommand
            | Self::MissingArgument
            | Self::NoActiveCommand
            | Self::TokenTooLong
            | Self::TooManyArguments
            | Self::TooManyStages
            | Self::UnknownArgument
            | Self::UnterminatedQuote => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Text<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    len: u16,
}

impl<const CAPACITY: usize> Text<CAPACITY> {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, Error> {
        let mut text = Self::empty();
        text.push_str(value)?;
        Ok(text)
    }

    pub fn push_str(&mut self, value: &str) -> Result<(), Error> {
        let start = self.len as usize;
        let end = start.checked_add(value.len()).ok_or(Error::Capacity)?;
        if end > CAPACITY || end > u16::MAX as usize {
            return Err(Error::TokenTooLong)
        }
        self.bytes[start..end].copy_from_slice(value.as_bytes());
        self.len = end as u16;
        Ok(())
    }

    pub fn push_char(&mut self, value: char) -> Result<(), Error> {
        let mut encoded = [0; 4];
        self.push_str(value.encode_utf8(&mut encoded))
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len()])
            .expect("shell text invariant")
    }

    pub fn clear(&mut self) {
        self.len = 0
    }
}

impl<const CAPACITY: usize> core::fmt::Debug for Text<CAPACITY> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("Text").field(&self.as_str()).finish()
    }
}

impl<const CAPACITY: usize> core::fmt::Write for Text<CAPACITY> {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        self.push_str(value).map_err(|_| core::fmt::Error)
    }
}
