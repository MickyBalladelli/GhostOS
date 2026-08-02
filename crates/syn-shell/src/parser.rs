use synos_system_model::{
    LogicalName,
    command::{ArgumentKind, ArgumentSpec, CommandSpec, MAX_COMMAND_ARGUMENTS},
};

use crate::{Error, MAX_TOKEN_BYTES, Text};

pub const DEFAULT_REGISTRY_CAPACITY: usize = 64;
pub const MAX_PIPELINE_STAGES: usize = 8;
const MAX_STAGE_WORDS: usize = MAX_COMMAND_ARGUMENTS + 2;
const MAX_COMMAND_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RouteId(u16);

impl RouteId {
    pub const fn new(raw: u16) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Boolean(bool),
    Integer(i64),
    Text(Text<MAX_TOKEN_BYTES>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Argument {
    pub name: LogicalName,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandCall {
    pub route: RouteId,
    pub command: LogicalName,
    arguments: [Option<Argument>; MAX_COMMAND_ARGUMENTS],
}

impl CommandCall {
    pub fn arguments(&self) -> impl Iterator<Item = Argument> + '_ {
        self.arguments.iter().flatten().copied()
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.arguments()
            .find(|argument| argument.name.as_str().eq_ignore_ascii_case(name))
            .map(|argument| argument.value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Program {
    stages: [Option<CommandCall>; MAX_PIPELINE_STAGES],
    stage_count: u8,
    pub background: bool,
}

impl Program {
    pub fn stages(&self) -> impl Iterator<Item = CommandCall> + '_ {
        self.stages.iter().flatten().copied()
    }

    pub const fn stage_count(&self) -> usize {
        self.stage_count as usize
    }

    pub fn stage(&self, index: usize) -> Option<CommandCall> {
        self.stages.get(index).copied().flatten()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandRegistration {
    pub spec: CommandSpec,
    pub route: RouteId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSuggestions<const CAPACITY: usize> {
    names: [Option<LogicalName>; CAPACITY],
    count: usize,
}

impl<const CAPACITY: usize> CommandSuggestions<CAPACITY> {
    fn new() -> Self {
        Self {
            names: [None; CAPACITY],
            count: 0,
        }
    }

    pub fn commands(&self) -> impl Iterator<Item = LogicalName> + '_ {
        self.names[..self.count].iter().flatten().copied()
    }

    fn push(&mut self, name: LogicalName) -> Result<(), Error> {
        let slot = self
            .names
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(name);
        self.count += 1;
        Ok(())
    }
}

pub struct CommandRegistry<const CAPACITY: usize = DEFAULT_REGISTRY_CAPACITY> {
    commands: [Option<CommandRegistration>; CAPACITY],
    command_count: usize,
}

impl<const CAPACITY: usize> CommandRegistry<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            commands: [None; CAPACITY],
            command_count: 0,
        }
    }

    pub fn register(&mut self, spec: CommandSpec, route: RouteId) -> Result<(), Error> {
        if self.commands[..self.command_count].iter().flatten().any(|entry| {
            entry
                .spec
                .name
                .as_str()
                .eq_ignore_ascii_case(spec.name.as_str())
        }) {
            return Err(Error::InvalidValue)
        }
        if self.command_count == CAPACITY {
            return Err(Error::Capacity)
        }
        self.commands[self.command_count] = Some(CommandRegistration { spec, route });
        self.command_count += 1;
        Ok(())
    }

    /// Commands in stable registration order for schema reflection.
    pub fn registrations(&self) -> impl Iterator<Item = CommandRegistration> + '_ {
        self.commands[..self.command_count].iter().flatten().copied()
    }

    pub fn suggestions(
        &self,
        input: &str,
    ) -> Result<CommandSuggestions<CAPACITY>, Error> {
        let command_name = self.command_prefix(input)?;

        let mut suggestions = CommandSuggestions::new();
        for entry in self.commands[..self.command_count].iter().flatten() {
            if starts_with_ignore_ascii_case(
                entry.spec.name.as_str(),
                command_name.as_str(),
            ) {
                suggestions.push(entry.spec.name)?
            }
        }
        Ok(suggestions)
    }

    pub fn unique_suggestion(
        &self,
        input: &str,
    ) -> Result<Option<&LogicalName>, Error> {
        let command_name = self.command_prefix(input)?;
        let mut match_name = None;
        let mut match_count = 0;
        for entry in self.commands[..self.command_count].iter().flatten() {
            if starts_with_ignore_ascii_case(
                entry.spec.name.as_str(),
                command_name.as_str(),
            ) {
                match_count += 1;
                match_name = Some(&entry.spec.name);
            }
        }
        Ok(if match_count == 1 { match_name } else { None })
    }

    fn command_prefix(&self, input: &str) -> Result<Text<MAX_COMMAND_NAME_BYTES>, Error> {
        let mut lexer = Lexer::new(input);
        let mut words: [Option<Text<MAX_TOKEN_BYTES>>; 2] = [None; 2];
        let mut word_count = 0;

        while let Some(lexeme) = lexer.next()? {
            match lexeme {
                Lexeme::Word(word) => {
                    if word_count == words.len() {
                        break
                    }
                    words[word_count] = Some(word);
                    word_count += 1
                }
                Lexeme::Pipe | Lexeme::Background => break,
            }
        }

        let first = words[0].ok_or(Error::InvalidSyntax)?;
        let mut command_name = Text::<MAX_COMMAND_NAME_BYTES>::empty();
        if first.as_str().eq_ignore_ascii_case("SHOW")
            || first.as_str().eq_ignore_ascii_case("SHO")
            || first.as_str().eq_ignore_ascii_case("TOP")
            || first.as_str().eq_ignore_ascii_case("SET")
        {
            let object = words[1].ok_or(Error::MissingArgument)?;
            let noun = object
                .as_str()
                .split_once('/')
                .map_or(object.as_str(), |(noun, _)| noun);
            let prefix = if first.as_str().eq_ignore_ascii_case("TOP") {
                "TOP-"
            } else if first.as_str().eq_ignore_ascii_case("SET") {
                "SET-"
            } else {
                "SHOW-"
            };
            command_name.push_str(prefix)?;
            command_name.push_str(noun)?;
        } else if let Some((verb, noun)) = first.as_str().split_once('/') {
            if noun.is_empty() {
                return Err(Error::InvalidSyntax)
            }
            if starts_with_ignore_ascii_case("ANALYZE", verb) {
                command_name.push_str("ANALYZE-")?;
                command_name.push_str(noun)?;
            } else {
                command_name.push_str(verb)?;
            }
        } else {
            command_name.push_str(first.as_str())?;
        }
        Ok(command_name)
    }

    pub fn parse(&self, input: &str) -> Result<Program, Error> {
        let mut lexer = Lexer::new(input);
        let mut words: [Option<Text<MAX_TOKEN_BYTES>>; MAX_STAGE_WORDS] =
            [None; MAX_STAGE_WORDS];
        let mut word_count = 0usize;
        let mut stages = [None; MAX_PIPELINE_STAGES];
        let mut stage_count = 0usize;
        let mut background = false;

        while let Some(lexeme) = lexer.next()? {
            match lexeme {
                Lexeme::Word(word) => {
                    if background {
                        return Err(Error::InvalidSyntax)
                    }
                    if word_count == MAX_STAGE_WORDS {
                        return Err(Error::TooManyArguments)
                    }
                    words[word_count] = Some(word);
                    word_count += 1
                }
                Lexeme::Pipe => {
                    if background || word_count == 0 {
                        return Err(Error::InvalidSyntax)
                    }
                    if stage_count == MAX_PIPELINE_STAGES {
                        return Err(Error::TooManyStages)
                    }
                    stages[stage_count] =
                        Some(self.parse_stage(&words, word_count)?);
                    stage_count += 1;
                    words = [None; MAX_STAGE_WORDS];
                    word_count = 0
                }
                Lexeme::Background => {
                    if word_count == 0 {
                        return Err(Error::InvalidSyntax)
                    }
                    background = true
                }
            }
        }

        if word_count == 0 {
            return Err(Error::InvalidSyntax)
        }
        if stage_count == MAX_PIPELINE_STAGES {
            return Err(Error::TooManyStages)
        }
        stages[stage_count] = Some(self.parse_stage(&words, word_count)?);
        stage_count += 1;
        Ok(Program {
            stages,
            stage_count: stage_count as u8,
            background,
        })
    }

    fn parse_stage(
        &self,
        words: &[Option<Text<MAX_TOKEN_BYTES>>; MAX_STAGE_WORDS],
        word_count: usize,
    ) -> Result<CommandCall, Error> {
        let verb = words[0].as_ref().ok_or(Error::InvalidSyntax)?;
        let mut command_name = Text::<MAX_COMMAND_NAME_BYTES>::empty();
        let mut first_argument = 1usize;
        let mut attached: Option<Text<MAX_TOKEN_BYTES>> = None;

        if verb.as_str().eq_ignore_ascii_case("SHOW")
            || verb.as_str().eq_ignore_ascii_case("SHO")
            || verb.as_str().eq_ignore_ascii_case("TOP")
            || verb.as_str().eq_ignore_ascii_case("SET")
        {
            if word_count < 2 {
                return Err(Error::MissingArgument)
            }
            let object = words[1].as_ref().ok_or(Error::InvalidSyntax)?;
            let (noun, qualifiers) = match object.as_str().split_once('/') {
                Some((noun, rest)) => (noun, Some(rest)),
                None => (object.as_str(), None),
            };
            if noun.is_empty() {
                return Err(Error::InvalidSyntax)
            }
            let prefix = if verb.as_str().eq_ignore_ascii_case("TOP") {
                "TOP-"
            } else if verb.as_str().eq_ignore_ascii_case("SET") {
                "SET-"
            } else {
                "SHOW-"
            };
            command_name.push_str(prefix)?;
            command_name.push_str(noun)?;
            first_argument = 2;
            attached = qualifiers.map(Text::new).transpose()?
        } else if let Some((verb_name, qualifier)) = verb.as_str().split_once('/') {
            if qualifier.is_empty() {
                return Err(Error::InvalidSyntax)
            }
            if starts_with_ignore_ascii_case("ANALYZE", verb_name) {
                command_name.push_str("ANALYZE-")?;
                command_name.push_str(qualifier)?
            } else {
                command_name.push_str(verb_name)?;
                attached = Some(Text::new(qualifier)?)
            }
        } else {
            command_name.push_str(verb.as_str())?
        }

        if command_name.as_str().eq_ignore_ascii_case("LN") {
            command_name = Text::new("LINK")?;
        } else if command_name.as_str().eq_ignore_ascii_case("LINKS") {
            command_name = Text::new("SHOW-LINKS")?;
        }
        let registration = self.find_registration(command_name.as_str())?;
        let mut arguments = [None; MAX_COMMAND_ARGUMENTS];
        let mut positional = 0usize;

        if let Some(attached) = attached {
            for qualifier in attached.as_str().split('/') {
                if qualifier.is_empty() {
                    return Err(Error::InvalidSyntax)
                }
                self.insert_qualifier(
                    &registration.spec,
                    qualifier,
                    &mut arguments,
                )?
            }
        }

        for word in words[first_argument..word_count].iter().flatten() {
            let raw = word.as_str();
            let long_qualifier = raw.strip_prefix("--");
            let slash_qualifier = raw
                .strip_prefix('/')
                .filter(|qualifier| is_known_qualifier(&registration.spec, qualifier));
            if let Some(qualifier) = long_qualifier.or(slash_qualifier) {
                self.insert_qualifier(
                    &registration.spec,
                    qualifier,
                    &mut arguments,
                )?
            } else if raw.starts_with('/')
                && registration
                    .spec
                    .arguments()
                    .filter(|argument| argument.positional)
                    .nth(positional)
                    .is_none()
            {
                self.insert_qualifier(
                    &registration.spec,
                    raw.strip_prefix('/').ok_or(Error::InvalidSyntax)?,
                    &mut arguments,
                )?
            } else {
                let spec = registration
                    .spec
                    .arguments()
                    .filter(|argument| argument.positional)
                    .nth(positional)
                    .ok_or(Error::TooManyArguments)?;
                positional += 1;
                insert_argument(&mut arguments, spec, raw)?
            }
        }

        if registration.spec.arguments().any(|spec| {
            spec.required
                && !arguments
                    .iter()
                    .flatten()
                    .any(|value| names_equal(value.name, spec.name))
        }) {
            return Err(Error::MissingArgument)
        }

        Ok(CommandCall {
            route: registration.route,
            command: registration.spec.name,
            arguments,
        })
    }

    fn insert_qualifier(
        &self,
        command: &CommandSpec,
        qualifier: &str,
        arguments: &mut [Option<Argument>; MAX_COMMAND_ARGUMENTS],
    ) -> Result<(), Error> {
        let (name, explicit) = qualifier
            .split_once('=')
            .map_or((qualifier, None), |(name, value)| (name, Some(value)));
        let direct = command
            .arguments()
            .find(|spec| spec.name.as_str().eq_ignore_ascii_case(name));
        let (spec, raw) = if let Some(spec) = direct {
            let raw = match (spec.kind, explicit) {
                (ArgumentKind::Boolean, None) => "true",
                (_, Some(value)) if !value.is_empty() => value,
                _ => return Err(Error::InvalidValue),
            };
            (spec, raw)
        } else if let Some(positive) = name.strip_prefix("NO") {
            let spec = command
                .arguments()
                .find(|spec| {
                    spec.kind == ArgumentKind::Boolean
                        && spec.name.as_str().eq_ignore_ascii_case(positive)
                })
                .ok_or(Error::UnknownArgument)?;
            if explicit.is_some() {
                return Err(Error::InvalidValue)
            }
            (spec, "false")
        } else {
            return Err(Error::UnknownArgument)
        };
        insert_argument(arguments, spec, raw)
    }

    fn find_registration(
        &self,
        command_name: &str,
    ) -> Result<&CommandRegistration, Error> {
        let mut exact = None;
        let mut prefix = None;
        let mut ambiguous = false;

        for entry in self.commands[..self.command_count].iter().flatten() {
            let name = entry.spec.name.as_str();
            if name.eq_ignore_ascii_case(command_name) {
                exact = Some(entry);
            } else if starts_with_ignore_ascii_case(name, command_name) {
                if prefix.is_some() {
                    ambiguous = true;
                } else {
                    prefix = Some(entry);
                }
            }
        }

        if let Some(entry) = exact {
            Ok(entry)
        } else if ambiguous {
            Err(Error::AmbiguousCommand)
        } else {
            prefix.ok_or(Error::UnknownCommand)
        }
    }
}

fn is_known_qualifier(command: &CommandSpec, qualifier: &str) -> bool {
    let (name, _) = qualifier
        .split_once('=')
        .map_or((qualifier, None), |(name, value)| (name, Some(value)));
    command
        .arguments()
        .any(|spec| spec.name.as_str().eq_ignore_ascii_case(name))
        || name.strip_prefix("NO").is_some_and(|positive| {
            command.arguments().any(|spec| {
                spec.kind == ArgumentKind::Boolean
                    && spec.name.as_str().eq_ignore_ascii_case(positive)
            })
        })
}

impl<const CAPACITY: usize> Default for CommandRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn insert_argument(
    arguments: &mut [Option<Argument>; MAX_COMMAND_ARGUMENTS],
    spec: ArgumentSpec,
    raw: &str,
) -> Result<(), Error> {
    if arguments
        .iter()
        .flatten()
        .any(|value| names_equal(value.name, spec.name))
    {
        return Err(Error::InvalidValue)
    }
    let value = match spec.kind {
        ArgumentKind::Boolean => match raw {
            value
                if value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
                    || value == "1" =>
            {
                Value::Boolean(true)
            }
            value
                if value.eq_ignore_ascii_case("false")
                    || value.eq_ignore_ascii_case("no")
                    || value == "0" =>
            {
                Value::Boolean(false)
            }
            _ => return Err(Error::InvalidValue),
        },
        ArgumentKind::Integer => Value::Integer(
            raw.parse::<i64>().map_err(|_| Error::InvalidValue)?,
        ),
        ArgumentKind::Text => Value::Text(Text::new(raw)?),
    };
    let slot = arguments
        .iter_mut()
        .find(|entry| entry.is_none())
        .ok_or(Error::TooManyArguments)?;
    *slot = Some(Argument {
        name: spec.name,
        value,
    });
    Ok(())
}

fn names_equal(left: LogicalName, right: LogicalName) -> bool {
    left.as_str().eq_ignore_ascii_case(right.as_str())
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lexeme {
    Word(Text<MAX_TOKEN_BYTES>),
    Pipe,
    Background,
}

struct Lexer<'a> {
    input: &'a str,
    offset: usize,
    stopped: bool,
}

impl<'a> Lexer<'a> {
    const fn new(input: &'a str) -> Self {
        Self {
            input,
            offset: 0,
            stopped: false,
        }
    }

    fn next(&mut self) -> Result<Option<Lexeme>, Error> {
        if self.stopped {
            return Ok(None)
        }
        self.skip_whitespace();
        let Some(current) = self.peek() else {
            return Ok(None)
        };
        match current {
            '!' => {
                self.stopped = true;
                Ok(None)
            }
            '|' => {
                self.advance(current);
                Ok(Some(Lexeme::Pipe))
            }
            '&' => {
                self.advance(current);
                Ok(Some(Lexeme::Background))
            }
            _ => self.word().map(|word| Some(Lexeme::Word(word))),
        }
    }

    fn word(&mut self) -> Result<Text<MAX_TOKEN_BYTES>, Error> {
        let mut word = Text::empty();
        let mut quote = None;

        while let Some(current) = self.peek() {
            if current == '\\' {
                return Err(Error::InvalidSyntax)
            }
            if let Some(expected) = quote {
                self.advance(current);
                if current == expected {
                    quote = None
                } else {
                    word.push_char(current)?
                }
                continue
            }
            if matches!(current, '\'' | '"') {
                quote = Some(current);
                self.advance(current);
                continue
            }
            if current.is_ascii_whitespace() || matches!(current, '|' | '&') {
                break
            }
            word.push_char(current)?;
            self.advance(current)
        }
        if quote.is_some() {
            return Err(Error::UnterminatedQuote)
        }
        if word.is_empty() {
            return Err(Error::InvalidSyntax)
        }
        Ok(word)
    }

    fn skip_whitespace(&mut self) {
        while let Some(current) = self.peek() {
            if !current.is_ascii_whitespace() {
                break
            }
            self.advance(current)
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn advance(&mut self, current: char) {
        self.offset += current.len_utf8()
    }
}
