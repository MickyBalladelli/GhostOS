use core::fmt::Write;

use ghostos_auth::{CapabilityCaveat, CapabilityLogicalNames, CryptographicCapability};
use ghostos_kernel::{AddressSpaceId, CapabilityHandle};
use ghostos_status::Status;
use ghostos_system_model::{
    LogicalName,
    logical::{LogicalError, LogicalScope, LogicalTarget, Principal},
};

use crate::{Error, MAX_SCRIPT_LINE_BYTES, MAX_SYMBOL_TEXT_BYTES, MAX_SYMBOLS, Text};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionIdentity {
    pub process: u64,
    pub job: Option<u64>,
    pub group: Option<u64>,
}

impl ExecutionIdentity {
    pub const fn new(process: u64, job: Option<u64>, group: Option<u64>) -> Option<Self> {
        if process == 0 {
            None
        } else {
            Some(Self {
                process,
                job,
                group,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolValue {
    Boolean(bool),
    Capability(CryptographicCapability),
    Integer(i64),
    Status(Status),
    Text(Text<MAX_SYMBOL_TEXT_BYTES>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Symbol {
    name: LogicalName,
    value: SymbolValue,
}

pub struct SymbolTable<const CAPACITY: usize = MAX_SYMBOLS> {
    symbols: [Option<Symbol>; CAPACITY],
}

impl<const CAPACITY: usize> SymbolTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            symbols: [None; CAPACITY],
        }
    }

    pub fn define(&mut self, name: &str, value: SymbolValue) -> Result<(), Error> {
        let name = LogicalName::new(name).map_err(|_| Error::InvalidName)?;
        if let Some(symbol) = self
            .symbols
            .iter_mut()
            .flatten()
            .find(|symbol| names_equal(symbol.name, name))
        {
            symbol.value = value;
            return Ok(());
        }
        let slot = self
            .symbols
            .iter_mut()
            .find(|symbol| symbol.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(Symbol { name, value });
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<SymbolValue> {
        self.symbols
            .iter()
            .flatten()
            .find(|symbol| symbol.name.as_str().eq_ignore_ascii_case(name))
            .map(|symbol| symbol.value)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), Error> {
        let slot = self
            .symbols
            .iter_mut()
            .find(|symbol| {
                symbol.is_some_and(|symbol| symbol.name.as_str().eq_ignore_ascii_case(name))
            })
            .ok_or(Error::InvalidName)?;
        *slot = None;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, SymbolValue)> + '_ {
        self.symbols
            .iter()
            .flatten()
            .map(|symbol| (symbol.name.as_str(), symbol.value))
    }

    pub fn expand(
        &self,
        input: &str,
        status: Status,
    ) -> Result<Text<MAX_SCRIPT_LINE_BYTES>, Error> {
        let mut output = Text::empty();
        let mut cursor = 0;

        while cursor < input.len() {
            let remainder = &input[cursor..];
            if remainder
                .get(..7)
                .is_some_and(|value| value.eq_ignore_ascii_case("$STATUS"))
            {
                write!(&mut output, "{}", status.raw()).map_err(|_| Error::LineTooLong)?;
                cursor += 7;
                continue;
            }
            if let Some(name_start) = remainder.strip_prefix("${") {
                let end = name_start.find('}').ok_or(Error::InvalidValue)?;
                let name = &name_start[..end];
                let value = self.get(name).ok_or(Error::InvalidName)?;
                write_symbol(&mut output, value)?;
                cursor += end + 3;
                continue;
            }
            let character = remainder.chars().next().ok_or(Error::InvalidValue)?;
            output
                .push_char(character)
                .map_err(|_| Error::LineTooLong)?;
            cursor += character.len_utf8();
        }
        Ok(output)
    }
}

impl<const CAPACITY: usize> Default for SymbolTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn write_symbol(output: &mut Text<MAX_SCRIPT_LINE_BYTES>, value: SymbolValue) -> Result<(), Error> {
    match value {
        SymbolValue::Boolean(value) => output
            .push_str(if value { "true" } else { "false" })
            .map_err(|_| Error::LineTooLong),
        SymbolValue::Integer(value) => write!(output, "{value}").map_err(|_| Error::LineTooLong),
        SymbolValue::Status(value) => {
            write!(output, "{}", value.raw()).map_err(|_| Error::LineTooLong)
        }
        SymbolValue::Text(value) => output
            .push_str(value.as_str())
            .map_err(|_| Error::LineTooLong),
        SymbolValue::Capability(_) => Err(Error::InvalidCapability),
    }
}

pub trait LogicalNameControl {
    fn define(
        &mut self,
        scope: LogicalScope,
        name: &str,
        target: LogicalTarget,
    ) -> Result<(), LogicalError>;

    fn delete(&mut self, scope: LogicalScope, name: &str) -> Result<(), LogicalError>;
}

pub struct NoLogicalNames;

impl LogicalNameControl for NoLogicalNames {
    fn define(
        &mut self,
        _scope: LogicalScope,
        _name: &str,
        _target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        Err(LogicalError::AccessDenied)
    }

    fn delete(&mut self, _scope: LogicalScope, _name: &str) -> Result<(), LogicalError> {
        Err(LogicalError::AccessDenied)
    }
}

/// Binds script logical-name verbs to the capability-checked system service.
pub struct AuthorizedLogicalNames<
    'a,
    const ENTRIES: usize,
    const ACLS: usize,
    const CAPABILITIES: usize,
> {
    names: &'a mut CapabilityLogicalNames<ENTRIES, ACLS, CAPABILITIES>,
    caller_space: AddressSpaceId,
    authority: CapabilityHandle,
    caller: Principal,
}

impl<'a, const ENTRIES: usize, const ACLS: usize, const CAPABILITIES: usize>
    AuthorizedLogicalNames<'a, ENTRIES, ACLS, CAPABILITIES>
{
    pub const fn new(
        names: &'a mut CapabilityLogicalNames<ENTRIES, ACLS, CAPABILITIES>,
        caller_space: AddressSpaceId,
        authority: CapabilityHandle,
        caller: Principal,
    ) -> Self {
        Self {
            names,
            caller_space,
            authority,
            caller,
        }
    }
}

impl<const ENTRIES: usize, const ACLS: usize, const CAPABILITIES: usize> LogicalNameControl
    for AuthorizedLogicalNames<'_, ENTRIES, ACLS, CAPABILITIES>
{
    fn define(
        &mut self,
        scope: LogicalScope,
        name: &str,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        self.names.define(
            self.caller_space,
            self.authority,
            self.caller,
            scope,
            name,
            target,
        )
    }

    fn delete(&mut self, scope: LogicalScope, name: &str) -> Result<(), LogicalError> {
        self.names
            .delete(self.caller_space, self.authority, self.caller, scope, name)
    }
}

pub struct ScriptContext<LogicalNames, const SYMBOLS: usize = MAX_SYMBOLS> {
    pub identity: ExecutionIdentity,
    pub logical_names: LogicalNames,
    pub symbols: SymbolTable<SYMBOLS>,
}

impl<LogicalNames, const SYMBOLS: usize> ScriptContext<LogicalNames, SYMBOLS> {
    pub const fn new(identity: ExecutionIdentity, logical_names: LogicalNames) -> Self {
        Self {
            identity,
            logical_names,
            symbols: SymbolTable::new(),
        }
    }

    pub fn attenuate(
        &mut self,
        source: &str,
        destination: &str,
        caveat: CapabilityCaveat,
    ) -> Result<(), Error> {
        let SymbolValue::Capability(capability) =
            self.symbols.get(source).ok_or(Error::InvalidCapability)?
        else {
            return Err(Error::InvalidCapability);
        };
        let attenuated = capability.attenuate(caveat)?;
        self.symbols
            .define(destination, SymbolValue::Capability(attenuated))
    }
}

fn names_equal(left: LogicalName, right: LogicalName) -> bool {
    left.as_str().eq_ignore_ascii_case(right.as_str())
}
