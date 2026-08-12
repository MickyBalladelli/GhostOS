use crate::{Error as ModelError, LogicalName};
use crate::performance::PerformanceDiagnostics;
use synos_status::{IntoStatus, Severity, Status, facility};

pub const MAX_COMMAND_ARGUMENTS: usize = 16;
pub const DEFAULT_COMMAND_CAPACITY: usize = 64;
pub const MAX_OUTPUT_FIELDS: usize = 32;
pub const MAX_OUTPUT_TEXT_BYTES: usize = 192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentKind {
    Boolean,
    Integer,
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArgumentSpec {
    pub name: LogicalName,
    pub kind: ArgumentKind,
    pub required: bool,
    pub positional: bool,
}

impl ArgumentSpec {
    pub fn new(
        name: &str,
        kind: ArgumentKind,
        required: bool,
        positional: bool,
    ) -> Result<Self, CommandError> {
        Ok(Self {
            name: LogicalName::new(name).map_err(map_name_error)?,
            kind,
            required,
            positional,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub name: LogicalName,
    arguments: [Option<ArgumentSpec>; MAX_COMMAND_ARGUMENTS],
}

impl CommandSpec {
    pub fn new(name: &str, arguments: &[ArgumentSpec]) -> Result<Self, CommandError> {
        if arguments.len() > MAX_COMMAND_ARGUMENTS {
            return Err(CommandError::TooManyArguments);
        }
        let mut stored: [Option<ArgumentSpec>; MAX_COMMAND_ARGUMENTS] =
            [None; MAX_COMMAND_ARGUMENTS];
        for (index, argument) in arguments.iter().copied().enumerate() {
            if stored[..index]
                .iter()
                .flatten()
                .any(|existing| names_equal(existing.name, argument.name))
            {
                return Err(CommandError::DuplicateArgument);
            }
            stored[index] = Some(argument)
        }
        Ok(Self {
            name: LogicalName::new(name).map_err(map_name_error)?,
            arguments: stored,
        })
    }

    pub fn arguments(&self) -> impl Iterator<Item = ArgumentSpec> + '_ {
        self.arguments.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentValue<'a> {
    Boolean(bool),
    Integer(i64),
    Text(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedArgument<'a> {
    pub name: LogicalName,
    pub value: ArgumentValue<'a>,
}

pub struct Invocation<'dictionary, 'input> {
    pub command: &'dictionary CommandSpec,
    values: [Option<ParsedArgument<'input>>; MAX_COMMAND_ARGUMENTS],
}

impl<'dictionary, 'input> Invocation<'dictionary, 'input> {
    pub fn arguments(&self) -> impl Iterator<Item = ParsedArgument<'input>> + '_ {
        self.values.iter().flatten().copied()
    }

    pub fn get(&self, name: &str) -> Option<ArgumentValue<'input>> {
        self.arguments()
            .find(|value| value.name.as_str().eq_ignore_ascii_case(name))
            .map(|value| value.value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    DictionaryFull,
    DuplicateArgument,
    DuplicateCommand,
    DuplicateValue,
    InvalidName,
    InvalidSyntax,
    InvalidValue,
    MissingArgument,
    TooManyArguments,
    UnknownArgument,
    UnknownCommand,
}

impl IntoStatus for CommandError {
    fn status(self) -> Status {
        let code = match self {
            Self::UnknownCommand => 1,
            Self::UnknownArgument => 2,
            Self::MissingArgument => 3,
            Self::InvalidValue => 4,
            Self::InvalidSyntax => 5,
            Self::DuplicateValue => 6,
            Self::DictionaryFull => 7,
            Self::DuplicateArgument => 8,
            Self::DuplicateCommand => 9,
            Self::InvalidName => 10,
            Self::TooManyArguments => 11,
        };
        Status::new(Severity::Error, facility::COMMAND, code, 0).expect("valid command status")
    }
}

/// Typed command dictionary. Parsing finishes before a handler is selected.
pub struct CommandDictionary<const CAPACITY: usize = DEFAULT_COMMAND_CAPACITY> {
    commands: [Option<CommandSpec>; CAPACITY],
}

impl<const CAPACITY: usize> CommandDictionary<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            commands: [None; CAPACITY],
        }
    }

    pub fn register(&mut self, command: CommandSpec) -> Result<(), CommandError> {
        if self
            .commands
            .iter()
            .flatten()
            .any(|existing| names_equal(existing.name, command.name))
        {
            return Err(CommandError::DuplicateCommand);
        }
        let slot = self
            .commands
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(CommandError::DictionaryFull)?;
        *slot = Some(command);
        Ok(())
    }

    pub fn parse<'dictionary, 'input>(
        &'dictionary self,
        input: &'input str,
    ) -> Result<Invocation<'dictionary, 'input>, CommandError> {
        let mut tokens = input.split_ascii_whitespace();
        let command_name = tokens.next().ok_or(CommandError::InvalidSyntax)?;
        let command = self
            .commands
            .iter()
            .flatten()
            .find(|command| command.name.as_str().eq_ignore_ascii_case(command_name))
            .ok_or(CommandError::UnknownCommand)?;
        let mut values: [Option<ParsedArgument<'input>>; MAX_COMMAND_ARGUMENTS] =
            [None; MAX_COMMAND_ARGUMENTS];
        let mut positional = 0;

        for token in tokens {
            let (spec, raw) = if token.starts_with('/') || token.starts_with("--") {
                let qualifier = token
                    .strip_prefix("--")
                    .or_else(|| token.strip_prefix('/'))
                    .ok_or(CommandError::InvalidSyntax)?;
                let (name, value) = qualifier
                    .split_once('=')
                    .map_or((qualifier, None), |(name, value)| (name, Some(value)));
                let spec = command
                    .arguments()
                    .find(|argument| argument.name.as_str().eq_ignore_ascii_case(name))
                    .ok_or(CommandError::UnknownArgument)?;
                let raw = match (spec.kind, value) {
                    (ArgumentKind::Boolean, None) => "true",
                    (_, Some(value)) if !value.is_empty() => value,
                    _ => return Err(CommandError::InvalidValue),
                };
                (spec, raw)
            } else {
                let spec = command
                    .arguments()
                    .filter(|argument| argument.positional)
                    .nth(positional)
                    .ok_or(CommandError::TooManyArguments)?;
                positional += 1;
                (spec, token)
            };
            if values
                .iter()
                .flatten()
                .any(|value| names_equal(value.name, spec.name))
            {
                return Err(CommandError::DuplicateValue);
            }
            let value = parse_value(spec.kind, raw)?;
            let slot = values
                .iter_mut()
                .find(|entry| entry.is_none())
                .ok_or(CommandError::TooManyArguments)?;
            *slot = Some(ParsedArgument {
                name: spec.name,
                value,
            })
        }

        if command.arguments().any(|argument| {
            argument.required
                && !values
                    .iter()
                    .flatten()
                    .any(|value| names_equal(value.name, argument.name))
        }) {
            return Err(CommandError::MissingArgument);
        }
        Ok(Invocation { command, values })
    }
}

impl<const CAPACITY: usize> Default for CommandDictionary<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputText {
    bytes: [u8; MAX_OUTPUT_TEXT_BYTES],
    len: u8,
}

impl OutputText {
    pub fn new(text: &str) -> Result<Self, CommandError> {
        let source = text.as_bytes();
        if source.len() > MAX_OUTPUT_TEXT_BYTES {
            return Err(CommandError::InvalidValue);
        }
        let mut bytes = [0; MAX_OUTPUT_TEXT_BYTES];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self {
            bytes,
            len: source.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("OutputText invariant")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputValue {
    Boolean(bool),
    Integer(i64),
    Status(Status),
    Text(OutputText),
    Unsigned(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputField {
    pub name: LogicalName,
    pub value: OutputValue,
}

pub struct OutputFields<'a> {
    fields: &'a [Option<OutputField>; MAX_OUTPUT_FIELDS],
    index: usize,
}

impl Iterator for OutputFields<'_> {
    type Item = OutputField;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < MAX_OUTPUT_FIELDS {
            let field = self.fields[self.index];
            self.index += 1;
            if field.is_some() {
                return field
            }
        }
        None
    }
}

pub struct StructuredOutput {
    status: Status,
    fields: [Option<OutputField>; MAX_OUTPUT_FIELDS],
}

impl StructuredOutput {
    pub const fn new(status: Status) -> Self {
        Self {
            status,
            fields: [None; MAX_OUTPUT_FIELDS],
        }
    }

    pub const fn status(&self) -> Status {
        self.status
    }

    pub fn fields(&self) -> OutputFields<'_> {
        OutputFields {
            fields: &self.fields,
            index: 0,
        }
    }

    pub fn insert(&mut self, name: &str, value: OutputValue) -> Result<(), CommandError> {
        let name = LogicalName::new(name).map_err(map_name_error)?;
        if let Some(field) = self
            .fields
            .iter_mut()
            .flatten()
            .find(|field| names_equal(field.name, name))
        {
            field.value = value;
            return Ok(());
        }
        let slot = self
            .fields
            .iter_mut()
            .find(|field| field.is_none())
            .ok_or(CommandError::TooManyArguments)?;
        *slot = Some(OutputField { name, value });
        Ok(())
    }

    pub fn insert_performance_diagnostics(
        &mut self,
        diagnostics: PerformanceDiagnostics,
    ) -> Result<(), CommandError> {
        self.insert("PERF-QUEUE-WAIT-US", OutputValue::Unsigned(diagnostics.queue_wait_us))?;
        self.insert("PERF-SERVICE-TIME-US", OutputValue::Unsigned(diagnostics.service_time_us))?;
        self.insert("PERF-RETRIES", OutputValue::Unsigned(diagnostics.retries as u64))?;
        self.insert("PERF-REQUEST-BYTES", OutputValue::Unsigned(diagnostics.request_bytes as u64))?;
        self.insert("PERF-RESPONSE-BYTES", OutputValue::Unsigned(diagnostics.response_bytes as u64))?;
        self.insert("PERF-TAIL-LATENCY-US", OutputValue::Unsigned(diagnostics.tail_latency_us))?;
        self.insert("PERF-BUDGET-EXCEEDED", OutputValue::Boolean(diagnostics.budget_exceeded))?;
        Ok(())
    }

    pub fn encoded_bytes(&self) -> u32 {
        self.fields()
            .map(|field| {
                field.name.as_str().len() as u32
                    + match field.value {
                        OutputValue::Boolean(_) => 1,
                        OutputValue::Integer(_) | OutputValue::Unsigned(_) => 8,
                        OutputValue::Status(_) => 4,
                        OutputValue::Text(value) => value.as_str().len(),
                    } as u32
            })
            .sum()
    }
}

fn parse_value(kind: ArgumentKind, raw: &str) -> Result<ArgumentValue<'_>, CommandError> {
    match kind {
        ArgumentKind::Boolean => match raw {
            value
                if value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
                    || value == "1" =>
            {
                Ok(ArgumentValue::Boolean(true))
            }
            value
                if value.eq_ignore_ascii_case("false")
                    || value.eq_ignore_ascii_case("no")
                    || value == "0" =>
            {
                Ok(ArgumentValue::Boolean(false))
            }
            _ => Err(CommandError::InvalidValue),
        },
        ArgumentKind::Integer => raw
            .parse::<i64>()
            .map(ArgumentValue::Integer)
            .map_err(|_| CommandError::InvalidValue),
        ArgumentKind::Text => Ok(ArgumentValue::Text(raw)),
    }
}

fn names_equal(left: LogicalName, right: LogicalName) -> bool {
    left.as_str().eq_ignore_ascii_case(right.as_str())
}

fn map_name_error(_: ModelError) -> CommandError {
    CommandError::InvalidName
}
