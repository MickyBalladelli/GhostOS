#![no_std]
#![forbid(unsafe_code)]

use ghostos_auth::TokenError;
use ghostos_status::{IntoStatus, Severity, Status, facility};
use ghostos_system_model::logical::LogicalError;

pub mod context;
pub mod parser;
pub mod reflection;
pub mod runtime;
pub mod sandbox;
pub mod wire;

pub use context::{
    AuthorizedLogicalNames, ExecutionIdentity, LogicalNameControl, NoLogicalNames, ScriptContext,
    SymbolTable, SymbolValue,
};
pub use parser::{
    CapabilityAttenuation, Condition, ErrorPolicy, ExitStatus, LogicalDefinition, LogicalDeletion,
    LogicalScopeSpec, Script, Statement, StatementKind, SymbolLiteral,
};
pub use reflection::ToolSchemaExporter;
pub use runtime::{ScriptEngine, ScriptEvent};
pub use sandbox::{
    CowSandbox, SandboxCommandHandler, SandboxDecision, SandboxExecutor, SandboxReceipt,
};
pub use ghostos_shell::Text;

pub const MAX_SCRIPT_STATEMENTS: usize = 64;
pub const MAX_SCRIPT_LINE_BYTES: usize = ghostos_shell::MAX_LINE_BYTES;
pub const MAX_SYMBOLS: usize = 64;
pub const MAX_SYMBOL_TEXT_BYTES: usize = 192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyRunning,
    Capacity,
    Inactive,
    InvalidCapability,
    InvalidCondition,
    InvalidDirective,
    InvalidName,
    InvalidScope,
    InvalidStatus,
    InvalidValue,
    LineTooLong,
    Logical(LogicalError),
    Filesystem(ghostos_ghostfs::Error),
    SchemaBufferTooSmall { required: usize },
    SchemaNameCollision,
    Shell(ghostos_shell::Error),
    Token(TokenError),
    UnterminatedQuote,
    WireBufferTooSmall,
    WireCorrupt,
    WireSchemaMismatch,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Logical(error) => error.status(),
            Self::Filesystem(error) => error.status(),
            Self::Shell(error) => error.status(),
            Self::Token(TokenError::AccessDenied | TokenError::RightsEscalation) => {
                Status::ACCESS_DENIED
            }
            Self::Token(TokenError::CaveatCapacity)
            | Self::Capacity
            | Self::SchemaBufferTooSmall { .. }
            | Self::WireBufferTooSmall => Status::NO_SPACE,
            Self::AlreadyRunning => Status::BUSY,
            Self::Inactive => Status::NOT_FOUND,
            Self::WireCorrupt => Status::CORRUPT,
            Self::WireSchemaMismatch => {
                Status::new(Severity::Error, facility::SCRIPT, 2, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::InvalidCapability
            | Self::InvalidCondition
            | Self::InvalidDirective
            | Self::InvalidName
            | Self::InvalidScope
            | Self::InvalidStatus
            | Self::InvalidValue
            | Self::SchemaNameCollision
            | Self::LineTooLong
            | Self::Token(TokenError::Invalid | TokenError::InvalidSignature)
            | Self::UnterminatedQuote => {
                Status::new(Severity::Error, facility::SCRIPT, 1, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
        }
    }
}

impl From<ghostos_shell::Error> for Error {
    fn from(error: ghostos_shell::Error) -> Self {
        Self::Shell(error)
    }
}

impl From<LogicalError> for Error {
    fn from(error: LogicalError) -> Self {
        Self::Logical(error)
    }
}

impl From<ghostos_ghostfs::Error> for Error {
    fn from(error: ghostos_ghostfs::Error) -> Self {
        Self::Filesystem(error)
    }
}

impl From<TokenError> for Error {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
