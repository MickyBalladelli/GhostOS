#![no_std]
#![forbid(unsafe_code)]

use synos_auth::TokenError;
use synos_status::{IntoStatus, Severity, Status, facility};
use synos_system_model::logical::LogicalError;

pub mod context;
pub mod parser;
pub mod runtime;
pub mod wire;

pub use context::{
    AuthorizedLogicalNames, ExecutionIdentity, LogicalNameControl, NoLogicalNames, ScriptContext,
    SymbolTable, SymbolValue,
};
pub use parser::{
    CapabilityAttenuation, Condition, ErrorPolicy, ExitStatus, LogicalDefinition, LogicalDeletion,
    LogicalScopeSpec, Script, Statement, StatementKind, SymbolLiteral,
};
pub use runtime::{ScriptEngine, ScriptEvent};
pub use syn_shell::Text;

pub const MAX_SCRIPT_STATEMENTS: usize = 64;
pub const MAX_SCRIPT_LINE_BYTES: usize = syn_shell::MAX_LINE_BYTES;
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
    Shell(syn_shell::Error),
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
            Self::Shell(error) => error.status(),
            Self::Token(TokenError::AccessDenied | TokenError::RightsEscalation) => {
                Status::ACCESS_DENIED
            }
            Self::Token(TokenError::CaveatCapacity) | Self::Capacity | Self::WireBufferTooSmall => {
                Status::NO_SPACE
            }
            Self::AlreadyRunning => Status::BUSY,
            Self::Inactive => Status::NOT_FOUND,
            Self::WireCorrupt => Status::CORRUPT,
            Self::WireSchemaMismatch => {
                Status::new(Severity::Error, facility::SCRIPT, 2, 0).expect("valid script status")
            }
            Self::InvalidCapability
            | Self::InvalidCondition
            | Self::InvalidDirective
            | Self::InvalidName
            | Self::InvalidScope
            | Self::InvalidStatus
            | Self::InvalidValue
            | Self::LineTooLong
            | Self::Token(TokenError::Invalid | TokenError::InvalidSignature)
            | Self::UnterminatedQuote => {
                Status::new(Severity::Error, facility::SCRIPT, 1, 0).expect("valid script status")
            }
        }
    }
}

impl From<syn_shell::Error> for Error {
    fn from(error: syn_shell::Error) -> Self {
        Self::Shell(error)
    }
}

impl From<LogicalError> for Error {
    fn from(error: LogicalError) -> Self {
        Self::Logical(error)
    }
}

impl From<TokenError> for Error {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
