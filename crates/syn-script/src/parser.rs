use synos_auth::TransportRights;
use synos_fabric::NodeId;
use synos_kernel::Rights;
use synos_status::Status;
use synos_system_model::{
    LogicalName,
    logical::{LogicalScope, LogicalTarget, LogicalTargetKind},
};

use crate::{Error, MAX_SCRIPT_LINE_BYTES, MAX_SCRIPT_STATEMENTS, MAX_SYMBOL_TEXT_BYTES, Text};

const MAX_DIRECTIVE_WORDS: usize = 20;
const MAX_DIRECTIVE_WORD_BYTES: usize = 192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Condition {
    Always,
    Success,
    Failure,
    StatusEquals(u32),
    StatusNotEquals(u32),
}

impl Condition {
    pub const fn matches(self, status: Status) -> bool {
        match self {
            Self::Always => true,
            Self::Success => status.is_success(),
            Self::Failure => !status.is_success(),
            Self::StatusEquals(expected) => status.raw() == expected,
            Self::StatusNotEquals(expected) => status.raw() != expected,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorPolicy {
    Exit,
    Continue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    Current,
    Value(Status),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalScopeSpec {
    Process(Option<u64>),
    Job(Option<u64>),
    Group(Option<u64>),
    System,
    Cluster,
}

impl LogicalScopeSpec {
    pub fn resolve(
        self,
        process: u64,
        job: Option<u64>,
        group: Option<u64>,
    ) -> Result<LogicalScope, Error> {
        match self {
            Self::Process(id) => Ok(LogicalScope::Process(id.unwrap_or(process))),
            Self::Job(id) => id.or(job).map(LogicalScope::Job).ok_or(Error::InvalidScope),
            Self::Group(id) => id
                .or(group)
                .map(LogicalScope::Group)
                .ok_or(Error::InvalidScope),
            Self::System => Ok(LogicalScope::System),
            Self::Cluster => Ok(LogicalScope::Cluster),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalDefinition {
    pub scope: LogicalScopeSpec,
    pub name: LogicalName,
    pub target: LogicalTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalDeletion {
    pub scope: LogicalScopeSpec,
    pub name: LogicalName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolLiteral {
    Boolean(bool),
    Integer(i64),
    Status(Status),
    Text(Text<MAX_SYMBOL_TEXT_BYTES>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityAttenuation {
    pub source: LogicalName,
    pub destination: LogicalName,
    pub rights: Rights,
    pub transports: Option<TransportRights>,
    pub expires_at_us: u64,
    pub subject: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatementKind {
    Attenuate(CapabilityAttenuation),
    DefineLogical(LogicalDefinition),
    DeleteLogical(LogicalDeletion),
    Exit(ExitStatus),
    Pipeline(Text<MAX_SCRIPT_LINE_BYTES>),
    SetErrorPolicy(ErrorPolicy),
    SetSymbol {
        name: LogicalName,
        value: SymbolLiteral,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Statement {
    pub condition: Condition,
    pub kind: StatementKind,
}

#[derive(Clone, Copy)]
pub struct Script<const CAPACITY: usize = MAX_SCRIPT_STATEMENTS> {
    statements: [Option<Statement>; CAPACITY],
    len: u8,
}

impl<const CAPACITY: usize> Script<CAPACITY> {
    pub fn compile(source: &str) -> Result<Self, Error> {
        let mut script = Self {
            statements: [None; CAPACITY],
            len: 0,
        };

        for source_line in source.lines() {
            let line = strip_comment(source_line)?.trim();
            let line = line.strip_prefix('$').map(str::trim_start).unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_SCRIPT_LINE_BYTES {
                return Err(Error::LineTooLong);
            }
            let statement = parse_statement(line)?;
            let slot = script
                .statements
                .iter_mut()
                .find(|statement| statement.is_none())
                .ok_or(Error::Capacity)?;
            *slot = Some(statement);
            script.len = script.len.checked_add(1).ok_or(Error::Capacity)?;
        }
        Ok(script)
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn statement(&self, index: usize) -> Option<Statement> {
        self.statements.get(index).copied().flatten()
    }

    pub fn statements(&self) -> impl Iterator<Item = Statement> + '_ {
        self.statements.iter().flatten().copied()
    }
}

fn parse_statement(line: &str) -> Result<Statement, Error> {
    let (condition, body) = parse_condition(line)?;
    let kind = if body.eq_ignore_ascii_case("SET ON") {
        StatementKind::SetErrorPolicy(ErrorPolicy::Exit)
    } else if body.eq_ignore_ascii_case("SET NOON") {
        StatementKind::SetErrorPolicy(ErrorPolicy::Continue)
    } else if starts_with_ignore_ascii_case(body, "ON ERROR THEN ") {
        match body[14..].trim() {
            action if action.eq_ignore_ascii_case("EXIT") => {
                StatementKind::SetErrorPolicy(ErrorPolicy::Exit)
            }
            action if action.eq_ignore_ascii_case("CONTINUE") => {
                StatementKind::SetErrorPolicy(ErrorPolicy::Continue)
            }
            _ => return Err(Error::InvalidDirective),
        }
    } else if starts_with_ignore_ascii_case(body, "SET SYMBOL ") {
        parse_set_symbol(body[11..].trim())?
    } else if starts_with_ignore_ascii_case(body, "DEFINE ") {
        StatementKind::DefineLogical(parse_define(body[7..].trim())?)
    } else if starts_with_ignore_ascii_case(body, "DEASSIGN ") {
        StatementKind::DeleteLogical(parse_delete(body[9..].trim())?)
    } else if starts_with_ignore_ascii_case(body, "DELETE LOGICAL ") {
        StatementKind::DeleteLogical(parse_delete(body[15..].trim())?)
    } else if starts_with_ignore_ascii_case(body, "ATTENUATE ") {
        StatementKind::Attenuate(parse_attenuation(body[10..].trim())?)
    } else if body.eq_ignore_ascii_case("EXIT") {
        StatementKind::Exit(ExitStatus::Current)
    } else if starts_with_ignore_ascii_case(body, "EXIT ") {
        StatementKind::Exit(parse_exit(body[5..].trim())?)
    } else if let Some(symbol) = parse_bare_symbol(body)? {
        symbol
    } else {
        StatementKind::Pipeline(Text::new(body).map_err(|_| Error::LineTooLong)?)
    };
    Ok(Statement { condition, kind })
}

fn parse_condition(line: &str) -> Result<(Condition, &str), Error> {
    if !starts_with_ignore_ascii_case(line, "IF ") {
        return Ok((Condition::Always, line));
    }
    let remainder = &line[3..];
    let then = find_ignore_ascii_case(remainder, " THEN ").ok_or(Error::InvalidCondition)?;
    let expression = remainder[..then].trim();
    let body = remainder[then + 6..].trim();
    if body.is_empty() {
        return Err(Error::InvalidCondition);
    }
    let condition = if expression.eq_ignore_ascii_case("$STATUS")
        || expression.eq_ignore_ascii_case("SUCCESS")
    {
        Condition::Success
    } else if expression.eq_ignore_ascii_case("FAILURE")
        || expression.eq_ignore_ascii_case("NOT $STATUS")
    {
        Condition::Failure
    } else {
        let mut words = expression.split_ascii_whitespace();
        let left = words.next().ok_or(Error::InvalidCondition)?;
        let operator = words.next().ok_or(Error::InvalidCondition)?;
        let right = words.next().ok_or(Error::InvalidCondition)?;
        if !left.eq_ignore_ascii_case("$STATUS") || words.next().is_some() {
            return Err(Error::InvalidCondition);
        }
        let raw = parse_u32(right)?;
        if operator.eq_ignore_ascii_case(".EQ.") || operator == "==" {
            Condition::StatusEquals(raw)
        } else if operator.eq_ignore_ascii_case(".NE.") || operator == "!=" {
            Condition::StatusNotEquals(raw)
        } else {
            return Err(Error::InvalidCondition);
        }
    };
    Ok((condition, body))
}

fn parse_set_symbol(input: &str) -> Result<StatementKind, Error> {
    let (name, raw) = input.split_once('=').ok_or(Error::InvalidDirective)?;
    let name = LogicalName::new(name.trim()).map_err(|_| Error::InvalidName)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::InvalidValue);
    }
    let quoted = raw
        .chars()
        .next()
        .is_some_and(|character| matches!(character, '\'' | '"'));
    let unquoted = unquote(raw)?;
    let value = if quoted {
        SymbolLiteral::Text(Text::new(unquoted).map_err(|_| Error::InvalidValue)?)
    } else if unquoted.eq_ignore_ascii_case("TRUE") || unquoted.eq_ignore_ascii_case("YES") {
        SymbolLiteral::Boolean(true)
    } else if unquoted.eq_ignore_ascii_case("FALSE") || unquoted.eq_ignore_ascii_case("NO") {
        SymbolLiteral::Boolean(false)
    } else if starts_with_ignore_ascii_case(unquoted, "STATUS(") && unquoted.ends_with(')') {
        let raw = parse_u32(&unquoted[7..unquoted.len() - 1])?;
        SymbolLiteral::Status(Status::from_raw(raw).ok_or(Error::InvalidStatus)?)
    } else if let Ok(value) = unquoted.parse::<i64>() {
        SymbolLiteral::Integer(value)
    } else {
        SymbolLiteral::Text(Text::new(unquoted).map_err(|_| Error::InvalidValue)?)
    };
    Ok(StatementKind::SetSymbol { name, value })
}

fn parse_bare_symbol(input: &str) -> Result<Option<StatementKind>, Error> {
    let Some((name, _)) = input.split_once('=') else {
        return Ok(None);
    };
    let name = name.trim();
    if name.contains(char::is_whitespace) || LogicalName::new(name).is_err() {
        return Ok(None);
    }
    parse_set_symbol(input).map(Some)
}

fn parse_define(input: &str) -> Result<LogicalDefinition, Error> {
    let words = tokenize(input)?;
    let mut scope = LogicalScopeSpec::Process(None);
    let mut kind = LogicalTargetKind::File;
    let mut name = None;
    let mut value = None;

    for word in words.iter().flatten() {
        let raw = word.as_str();
        if raw == "=" {
            continue;
        }
        if raw.starts_with('/') || raw.starts_with("--") {
            let qualifier = raw
                .strip_prefix("--")
                .or_else(|| raw.strip_prefix('/'))
                .ok_or(Error::InvalidDirective)?;
            if let Some(parsed) = parse_scope(qualifier)? {
                scope = parsed;
            } else if qualifier.eq_ignore_ascii_case("FILE") {
                kind = LogicalTargetKind::File;
            } else if qualifier.eq_ignore_ascii_case("DEVICE") {
                kind = LogicalTargetKind::Device;
            } else if qualifier.eq_ignore_ascii_case("CHANNEL")
                || qualifier.eq_ignore_ascii_case("IPC")
            {
                kind = LogicalTargetKind::IpcChannel;
            } else {
                return Err(Error::InvalidDirective);
            }
        } else if name.is_none() {
            name = Some(LogicalName::new(raw).map_err(|_| Error::InvalidName)?);
        } else if value.is_none() {
            value = Some(raw);
        } else {
            return Err(Error::InvalidDirective);
        }
    }
    let target =
        LogicalTarget::new(kind, value.ok_or(Error::InvalidValue)?).map_err(Error::Logical)?;
    Ok(LogicalDefinition {
        scope,
        name: name.ok_or(Error::InvalidName)?,
        target,
    })
}

fn parse_delete(input: &str) -> Result<LogicalDeletion, Error> {
    let words = tokenize(input)?;
    let mut scope = LogicalScopeSpec::Process(None);
    let mut name = None;
    for word in words.iter().flatten() {
        let raw = word.as_str();
        if raw.starts_with('/') || raw.starts_with("--") {
            let qualifier = raw
                .strip_prefix("--")
                .or_else(|| raw.strip_prefix('/'))
                .ok_or(Error::InvalidDirective)?;
            scope = parse_scope(qualifier)?.ok_or(Error::InvalidDirective)?;
        } else if name.is_none() {
            name = Some(LogicalName::new(raw).map_err(|_| Error::InvalidName)?);
        } else {
            return Err(Error::InvalidDirective);
        }
    }
    Ok(LogicalDeletion {
        scope,
        name: name.ok_or(Error::InvalidName)?,
    })
}

fn parse_attenuation(input: &str) -> Result<CapabilityAttenuation, Error> {
    let words = tokenize(input)?;
    let mut source = None;
    let mut destination = None;
    let mut saw_as = false;
    let mut rights = None;
    let mut transports = None;
    let mut expires_at_us = None;
    let mut subject = None;

    for word in words.iter().flatten() {
        let raw = word.as_str();
        if raw.eq_ignore_ascii_case("AS") {
            if source.is_none() || saw_as {
                return Err(Error::InvalidDirective);
            }
            saw_as = true;
            continue;
        }
        if raw.starts_with('/') || raw.starts_with("--") {
            let qualifier = raw
                .strip_prefix("--")
                .or_else(|| raw.strip_prefix('/'))
                .ok_or(Error::InvalidDirective)?;
            let (name, value) = qualifier.split_once('=').ok_or(Error::InvalidDirective)?;
            if name.eq_ignore_ascii_case("RIGHTS") {
                rights = Some(parse_rights(value)?);
            } else if name.eq_ignore_ascii_case("EXPIRES")
                || name.eq_ignore_ascii_case("EXPIRES_AT_US")
            {
                expires_at_us = Some(parse_u64(value)?);
            } else if name.eq_ignore_ascii_case("TRANSPORT")
                || name.eq_ignore_ascii_case("TRANSPORTS")
            {
                transports = Some(parse_transports(value)?);
            } else if name.eq_ignore_ascii_case("SUBJECT") {
                subject = Some(NodeId::new(parse_u32(value)?).ok_or(Error::InvalidValue)?);
            } else {
                return Err(Error::InvalidDirective);
            }
        } else if source.is_none() {
            source = Some(LogicalName::new(raw).map_err(|_| Error::InvalidName)?);
        } else if saw_as && destination.is_none() {
            destination = Some(LogicalName::new(raw).map_err(|_| Error::InvalidName)?);
        } else {
            return Err(Error::InvalidDirective);
        }
    }

    Ok(CapabilityAttenuation {
        source: source.ok_or(Error::InvalidCapability)?,
        destination: destination.ok_or(Error::InvalidCapability)?,
        rights: rights.ok_or(Error::InvalidCapability)?,
        transports,
        expires_at_us: expires_at_us.ok_or(Error::InvalidCapability)?,
        subject,
    })
}

fn parse_exit(input: &str) -> Result<ExitStatus, Error> {
    if input.eq_ignore_ascii_case("$STATUS") {
        return Ok(ExitStatus::Current);
    }
    let raw = parse_u32(input)?;
    Ok(ExitStatus::Value(
        Status::from_raw(raw).ok_or(Error::InvalidStatus)?,
    ))
}

fn parse_scope(qualifier: &str) -> Result<Option<LogicalScopeSpec>, Error> {
    let (name, raw_id) = qualifier
        .split_once('=')
        .map_or((qualifier, None), |(name, value)| (name, Some(value)));
    let id = raw_id.map(parse_u64).transpose()?;
    if name.eq_ignore_ascii_case("PROCESS") {
        Ok(Some(LogicalScopeSpec::Process(id)))
    } else if name.eq_ignore_ascii_case("JOB") {
        Ok(Some(LogicalScopeSpec::Job(id)))
    } else if name.eq_ignore_ascii_case("GROUP") {
        Ok(Some(LogicalScopeSpec::Group(id)))
    } else if name.eq_ignore_ascii_case("SYSTEM") && id.is_none() {
        Ok(Some(LogicalScopeSpec::System))
    } else if name.eq_ignore_ascii_case("CLUSTER") && id.is_none() {
        Ok(Some(LogicalScopeSpec::Cluster))
    } else {
        Ok(None)
    }
}

fn parse_rights(input: &str) -> Result<Rights, Error> {
    if let Ok(bits) = parse_u16(input) {
        let rights = Rights::from_bits(bits).ok_or(Error::InvalidCapability)?;
        return if rights.is_empty() {
            Err(Error::InvalidCapability)
        } else {
            Ok(rights)
        };
    }
    let mut rights = Rights::NONE;
    for raw in input.split(',') {
        let right = if raw.eq_ignore_ascii_case("READ") {
            Rights::READ
        } else if raw.eq_ignore_ascii_case("WRITE") {
            Rights::WRITE
        } else if raw.eq_ignore_ascii_case("EXECUTE") {
            Rights::EXECUTE
        } else if raw.eq_ignore_ascii_case("MAP") {
            Rights::MAP
        } else if raw.eq_ignore_ascii_case("CREATE") {
            Rights::CREATE
        } else if raw.eq_ignore_ascii_case("SEND") {
            Rights::SEND
        } else if raw.eq_ignore_ascii_case("RECEIVE") {
            Rights::RECEIVE
        } else if raw.eq_ignore_ascii_case("DELEGATE") {
            Rights::DELEGATE
        } else if raw.eq_ignore_ascii_case("REVOKE") {
            Rights::REVOKE
        } else {
            return Err(Error::InvalidCapability);
        };
        rights = rights.union(right);
    }
    if rights.is_empty() {
        Err(Error::InvalidCapability)
    } else {
        Ok(rights)
    }
}

fn parse_transports(input: &str) -> Result<TransportRights, Error> {
    let mut bits = 0;
    for raw in input.split(',') {
        if raw.eq_ignore_ascii_case("CXL") {
            bits |= TransportRights::CXL.bits();
        } else if raw.eq_ignore_ascii_case("LAYER2") || raw.eq_ignore_ascii_case("ETHERNET") {
            bits |= TransportRights::LAYER2.bits();
        } else {
            return Err(Error::InvalidCapability);
        }
    }
    TransportRights::from_bits(bits).ok_or(Error::InvalidCapability)
}

fn tokenize(
    input: &str,
) -> Result<[Option<Text<MAX_DIRECTIVE_WORD_BYTES>>; MAX_DIRECTIVE_WORDS], Error> {
    let mut words = [None; MAX_DIRECTIVE_WORDS];
    let mut count = 0;
    let mut cursor = 0;

    while cursor < input.len() {
        while cursor < input.len() {
            let character = input[cursor..]
                .chars()
                .next()
                .ok_or(Error::InvalidDirective)?;
            if !character.is_ascii_whitespace() {
                break;
            }
            cursor += character.len_utf8();
        }
        if cursor == input.len() {
            break;
        }
        if count == MAX_DIRECTIVE_WORDS {
            return Err(Error::Capacity);
        }
        let mut word = Text::empty();
        let mut quote = None;
        let mut escaped = false;
        while cursor < input.len() {
            let character = input[cursor..]
                .chars()
                .next()
                .ok_or(Error::InvalidDirective)?;
            if escaped {
                word.push_char(character).map_err(|_| Error::InvalidValue)?;
                cursor += character.len_utf8();
                escaped = false;
                continue;
            }
            if character == '\\' {
                cursor += 1;
                escaped = true;
                continue;
            }
            if let Some(expected) = quote {
                cursor += character.len_utf8();
                if character == expected {
                    quote = None;
                } else {
                    word.push_char(character).map_err(|_| Error::InvalidValue)?;
                }
                continue;
            }
            if matches!(character, '\'' | '"') {
                quote = Some(character);
                cursor += 1;
                continue;
            }
            if character.is_ascii_whitespace() {
                break;
            }
            word.push_char(character).map_err(|_| Error::InvalidValue)?;
            cursor += character.len_utf8();
        }
        if quote.is_some() || escaped {
            return Err(Error::UnterminatedQuote);
        }
        if word.is_empty() {
            return Err(Error::InvalidDirective);
        }
        words[count] = Some(word);
        count += 1;
    }
    Ok(words)
}

fn strip_comment(input: &str) -> Result<&str, Error> {
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(expected) = quote {
            if character == expected {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '!' {
            return Ok(&input[..offset]);
        }
    }
    if quote.is_some() || escaped {
        Err(Error::UnterminatedQuote)
    } else {
        Ok(input)
    }
}

fn unquote(input: &str) -> Result<&str, Error> {
    if let Some(first) = input.chars().next()
        && matches!(first, '\'' | '"')
    {
        if input.len() < 2 || !input.ends_with(first) {
            return Err(Error::UnterminatedQuote);
        }
        return Ok(&input[first.len_utf8()..input.len() - first.len_utf8()]);
    }
    Ok(input)
}

fn parse_u16(input: &str) -> Result<u16, Error> {
    parse_unsigned(input).and_then(|value| u16::try_from(value).map_err(|_| Error::InvalidValue))
}

fn parse_u32(input: &str) -> Result<u32, Error> {
    parse_unsigned(input).and_then(|value| u32::try_from(value).map_err(|_| Error::InvalidValue))
}

fn parse_u64(input: &str) -> Result<u64, Error> {
    parse_unsigned(input)
}

fn parse_unsigned(input: &str) -> Result<u64, Error> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .or_else(|| input.strip_prefix("%X"))
        .or_else(|| input.strip_prefix("%x"))
    {
        u64::from_str_radix(hex, 16).map_err(|_| Error::InvalidValue)
    } else {
        input.parse::<u64>().map_err(|_| Error::InvalidValue)
    }
}

fn starts_with_ignore_ascii_case(input: &str, prefix: &str) -> bool {
    input
        .get(..prefix.len())
        .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
}

fn find_ignore_ascii_case(input: &str, needle: &str) -> Option<usize> {
    input
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
