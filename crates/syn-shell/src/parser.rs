use synos_system_model::{
    command::{ArgumentKind, ArgumentSpec, CommandSpec, MAX_COMMAND_ARGUMENTS},
    LogicalName,
};

use crate::{Error, Text, MAX_TOKEN_BYTES};

pub const DEFAULT_REGISTRY_CAPACITY: usize = 64;
pub const MAX_PIPELINE_STAGES: usize = 8;
const MAX_STAGE_WORDS: usize = MAX_COMMAND_ARGUMENTS + 2;
const MAX_COMMAND_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RouteId(u16);

impl RouteId {
    pub const fn new(raw: u16) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
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

    pub fn get_text(&self, name: &str) -> Option<&str> {
        self.arguments
            .iter()
            .flatten()
            .find(|argument| argument.name.as_str().eq_ignore_ascii_case(name))
            .and_then(|argument| match &argument.value {
                Value::Text(value) => Some(value.as_str()),
                _ => None,
            })
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

    pub fn stage_ref(&self, index: usize) -> Option<&CommandCall> {
        self.stages.get(index).and_then(Option::as_ref)
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
        if self.commands[..self.command_count]
            .iter()
            .flatten()
            .any(|entry| {
                entry
                    .spec
                    .name
                    .as_str()
                    .eq_ignore_ascii_case(spec.name.as_str())
            })
        {
            return Err(Error::InvalidValue);
        }
        if self.command_count == CAPACITY {
            return Err(Error::Capacity);
        }
        self.commands[self.command_count] = Some(CommandRegistration { spec, route });
        self.command_count += 1;
        Ok(())
    }

    /// Commands in stable registration order for schema reflection.
    pub fn registrations(&self) -> impl Iterator<Item = CommandRegistration> + '_ {
        self.commands[..self.command_count]
            .iter()
            .flatten()
            .copied()
    }

    pub fn registration(&self, name: &str) -> Option<&CommandRegistration> {
        self.commands[..self.command_count]
            .iter()
            .flatten()
            .find(|registration| registration.spec.name.as_str().eq_ignore_ascii_case(name))
    }

    pub fn suggestions(&self, input: &str) -> Result<CommandSuggestions<CAPACITY>, Error> {
        let command_name = self.command_prefix(input)?;

        let mut suggestions = CommandSuggestions::new();
        for entry in self.commands[..self.command_count].iter().flatten() {
            if starts_with_ignore_ascii_case(entry.spec.name.as_str(), command_name.as_str()) {
                suggestions.push(entry.spec.name)?
            }
        }
        Ok(suggestions)
    }

    pub fn unique_suggestion(&self, input: &str) -> Result<Option<&LogicalName>, Error> {
        let command_name = self.command_prefix(input)?;
        let mut match_name = None;
        let mut match_count = 0;
        for entry in self.commands[..self.command_count].iter().flatten() {
            if starts_with_ignore_ascii_case(entry.spec.name.as_str(), command_name.as_str()) {
                match_count += 1;
                match_name = Some(&entry.spec.name);
            }
        }
        Ok(if match_count == 1 { match_name } else { None })
    }

    fn command_prefix(&self, input: &str) -> Result<Text<MAX_COMMAND_NAME_BYTES>, Error> {
        let mut lexer = Lexer::new(input);
        let mut words: [Option<Text<MAX_TOKEN_BYTES>>; 3] = [None; 3];
        let mut word_count = 0;

        while let Some(lexeme) = lexer.next()? {
            match lexeme {
                Lexeme::Word(word) => {
                    if word_count == words.len() {
                        break;
                    }
                    words[word_count] = Some(word);
                    word_count += 1
                }
                Lexeme::Pipe | Lexeme::Background => break,
            }
        }

        let first = words[0].ok_or(Error::InvalidSyntax)?;
        let mut command_name = Text::<MAX_COMMAND_NAME_BYTES>::empty();
        if first.as_str().eq_ignore_ascii_case("HELP") {
            command_name.push_str("HELP")?;
            if let Some(target) = words[1].as_ref() {
                command_name.clear();
                command_name.push_str(target.as_str())?;
                if matches!(
                    target.as_str(),
                    value if value.eq_ignore_ascii_case("SHOW")
                        || value.eq_ignore_ascii_case("TOP")
                        || value.eq_ignore_ascii_case("SET")
                ) {
                    if let Some(noun) = words[2].as_ref() {
                        command_name.push_char('-')?;
                        command_name.push_str(noun.as_str())?;
                    }
                }
            }
        } else if first.as_str().eq_ignore_ascii_case("SHOW")
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
        } else if let Some(cluster_command) =
            dcl_cluster_command(first.as_str(), words[1].as_ref().map(Text::as_str))
        {
            command_name.push_str(cluster_command)?;
        } else if let Some((verb, noun)) = first.as_str().split_once('/') {
            if noun.is_empty() {
                return Err(Error::InvalidSyntax);
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
        let mut words: [Option<Text<MAX_TOKEN_BYTES>>; MAX_STAGE_WORDS] = [None; MAX_STAGE_WORDS];
        let mut word_count = 0usize;
        let mut stages = [None; MAX_PIPELINE_STAGES];
        let mut stage_count = 0usize;
        let mut background = false;

        while let Some(lexeme) = lexer.next()? {
            match lexeme {
                Lexeme::Word(word) => {
                    if background {
                        return Err(Error::InvalidSyntax);
                    }
                    if word_count == MAX_STAGE_WORDS {
                        return Err(Error::TooManyArguments);
                    }
                    words[word_count] = Some(word);
                    word_count += 1
                }
                Lexeme::Pipe => {
                    if background || word_count == 0 {
                        return Err(Error::InvalidSyntax);
                    }
                    if stage_count == MAX_PIPELINE_STAGES {
                        return Err(Error::TooManyStages);
                    }
                    stages[stage_count] = Some(self.parse_stage(&words, word_count)?);
                    stage_count += 1;
                    words = [None; MAX_STAGE_WORDS];
                    word_count = 0
                }
                Lexeme::Background => {
                    if word_count == 0 {
                        return Err(Error::InvalidSyntax);
                    }
                    background = true
                }
            }
        }

        if word_count == 0 {
            return Err(Error::InvalidSyntax);
        }
        if stage_count == MAX_PIPELINE_STAGES {
            return Err(Error::TooManyStages);
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

        if let Some(cluster_command) =
            dcl_cluster_command(verb.as_str(), words[1].as_ref().map(Text::as_str))
        {
            let object = words[1].as_ref().ok_or(Error::MissingArgument)?;
            let (noun, qualifiers) = match object.as_str().split_once('/') {
                Some((noun, rest)) => (noun, Some(rest)),
                None => (object.as_str(), None),
            };
            if noun.is_empty() {
                return Err(Error::InvalidSyntax);
            }
            command_name.push_str(cluster_command)?;
            first_argument = 2;
            attached = qualifiers.map(Text::new).transpose()?
        } else if verb.as_str().eq_ignore_ascii_case("SHOW")
            || verb.as_str().eq_ignore_ascii_case("SHO")
            || verb.as_str().eq_ignore_ascii_case("TOP")
            || verb.as_str().eq_ignore_ascii_case("SET")
        {
            if word_count < 2 {
                return Err(Error::MissingArgument);
            }
            let object = words[1].as_ref().ok_or(Error::InvalidSyntax)?;
            let (noun, qualifiers) = match object.as_str().split_once('/') {
                Some((noun, rest)) => (noun, Some(rest)),
                None => (object.as_str(), None),
            };
            if noun.is_empty() {
                return Err(Error::InvalidSyntax);
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
                return Err(Error::InvalidSyntax);
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
        } else if command_name.as_str().eq_ignore_ascii_case("DEL")
            || command_name.as_str().eq_ignore_ascii_case("ERASE")
            || command_name.as_str().eq_ignore_ascii_case("RM")
        {
            command_name = Text::new("DELETE")?;
        } else if command_name.as_str().eq_ignore_ascii_case("RD") {
            command_name = Text::new("RMDIR")?;
        } else if command_name.as_str().eq_ignore_ascii_case("CLUSTER") {
            command_name = Text::new("SHOW-CLUSTER")?;
        } else if command_name.as_str().eq_ignore_ascii_case("LS-CLUSTERS") {
            command_name = Text::new("LIST-CLUSTERS")?;
        } else if command_name.as_str().eq_ignore_ascii_case("REMOVE-CLUSTER") {
            command_name = Text::new("DELETE-CLUSTER")?;
        }
        let registration = self.find_registration(command_name.as_str())?;
        let mut arguments = [None; MAX_COMMAND_ARGUMENTS];
        let mut positional = 0usize;

        if command_name.as_str().eq_ignore_ascii_case("HELP") && word_count > 1 {
            if word_count > 3 {
                return Err(Error::TooManyArguments);
            }
            let mut target = Text::<MAX_COMMAND_NAME_BYTES>::empty();
            target.push_str(words[1].as_ref().ok_or(Error::InvalidSyntax)?.as_str())?;
            if word_count == 3 {
                target.push_char('-')?;
                target.push_str(words[2].as_ref().ok_or(Error::InvalidSyntax)?.as_str())?;
            }
            if target.as_str().eq_ignore_ascii_case("CLUSTER") {
                target = Text::new("SHOW-CLUSTER")?;
            } else if target.as_str().eq_ignore_ascii_case("LS-CLUSTERS") {
                target = Text::new("LIST-CLUSTERS")?;
            } else if target.as_str().eq_ignore_ascii_case("REMOVE-CLUSTER") {
                target = Text::new("DELETE-CLUSTER")?;
            }
            let target_registration = self.find_registration(target.as_str())?;
            let target_argument = registration
                .spec
                .arguments()
                .find(|argument| argument.name.as_str().eq_ignore_ascii_case("COMMAND"))
                .ok_or(Error::TooManyArguments)?;
            insert_argument(
                &mut arguments,
                target_argument,
                target_registration.spec.name.as_str(),
            )?;
            first_argument = word_count;
        }

        if let Some(attached) = attached {
            for qualifier in attached.as_str().split('/') {
                if qualifier.is_empty() {
                    return Err(Error::InvalidSyntax);
                }
                self.insert_qualifier(&registration.spec, qualifier, &mut arguments)?
            }
        }

        for word in words[first_argument..word_count].iter().flatten() {
            let raw = word.as_str();
            let long_qualifier = raw.strip_prefix("--");
            let slash_qualifier = raw
                .strip_prefix('/')
                .filter(|qualifier| is_known_qualifier(&registration.spec, qualifier));
            if let Some(qualifier) = long_qualifier.or(slash_qualifier) {
                self.insert_qualifier(&registration.spec, qualifier, &mut arguments)?
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
            return Err(Error::MissingArgument);
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
                return Err(Error::InvalidValue);
            }
            (spec, "false")
        } else {
            return Err(Error::UnknownArgument);
        };
        insert_argument(arguments, spec, raw)
    }

    fn find_registration(&self, command_name: &str) -> Result<&CommandRegistration, Error> {
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

fn dcl_cluster_command(verb: &str, object: Option<&str>) -> Option<&'static str> {
    let object = object?.split_once('/').map_or(object?, |(noun, _)| noun);
    if (verb.eq_ignore_ascii_case("LIST") || verb.eq_ignore_ascii_case("LS"))
        && object.eq_ignore_ascii_case("CLUSTERS")
    {
        Some("LIST-CLUSTERS")
    } else if verb.eq_ignore_ascii_case("CREATE") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("CREATE-CLUSTER")
    } else if verb.eq_ignore_ascii_case("JOIN") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("JOIN-CLUSTER")
    } else if verb.eq_ignore_ascii_case("LEAVE") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("LEAVE-CLUSTER")
    } else if (verb.eq_ignore_ascii_case("REMOVE") || verb.eq_ignore_ascii_case("DELETE"))
        && object.eq_ignore_ascii_case("CLUSTER")
    {
        Some("DELETE-CLUSTER")
    } else if verb.eq_ignore_ascii_case("MODIFY") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("MODIFY-CLUSTER")
    } else if verb.eq_ignore_ascii_case("RENAME") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("RENAME-CLUSTER")
    } else if verb.eq_ignore_ascii_case("SET") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("SET-CLUSTER")
    } else if verb.eq_ignore_ascii_case("USE") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("USE-CLUSTER")
    } else if verb.eq_ignore_ascii_case("INVITE") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("INVITE-CLUSTER")
    } else if verb.eq_ignore_ascii_case("ACCEPT") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("ACCEPT-CLUSTER")
    } else if verb.eq_ignore_ascii_case("REJECT") && object.eq_ignore_ascii_case("CLUSTER") {
        Some("REJECT-CLUSTER")
    } else if verb.eq_ignore_ascii_case("REMOVE") && object.eq_ignore_ascii_case("FEDERATION") {
        Some("REMOVE-FEDERATION")
    } else if verb.eq_ignore_ascii_case("INVITE") && object.eq_ignore_ascii_case("NODE") {
        Some("INVITE-NODE")
    } else if verb.eq_ignore_ascii_case("ACCEPT") && object.eq_ignore_ascii_case("NODE") {
        Some("ACCEPT-NODE")
    } else if verb.eq_ignore_ascii_case("REJECT") && object.eq_ignore_ascii_case("NODE") {
        Some("REJECT-NODE")
    } else if verb.eq_ignore_ascii_case("REMOVE") && object.eq_ignore_ascii_case("NODE") {
        Some("REMOVE-NODE")
    } else if verb.eq_ignore_ascii_case("DRAIN") && object.eq_ignore_ascii_case("NODE") {
        Some("DRAIN-NODE")
    } else if verb.eq_ignore_ascii_case("FENCE") && object.eq_ignore_ascii_case("NODE") {
        Some("FENCE-NODE")
    } else if verb.eq_ignore_ascii_case("REJOIN") && object.eq_ignore_ascii_case("NODE") {
        Some("REJOIN-NODE")
    } else {
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{
        register_filesystem_commands, CREATE_FILE_ROUTE, DIRECTORY_ROUTE, EDIT_ROUTE, LINK_ROUTE,
        MKDIR_ROUTE, RMDIR_ROUTE, SET_DEFAULT_ROUTE, SHOW_DEFAULT_ROUTE, SHOW_LINKS_ROUTE,
        TYPE_ROUTE,
    };

    fn registry() -> CommandRegistry<16> {
        let mut registry = CommandRegistry::new();
        register_filesystem_commands(&mut registry).expect("register filesystem commands");
        registry
    }

    fn text(value: Option<Value>) -> Text<MAX_TOKEN_BYTES> {
        match value {
            Some(Value::Text(value)) => value,
            _ => panic!("expected text argument"),
        }
    }

    #[test]
    fn parses_link_aliases_and_versioned_source() {
        let registry = registry();
        let program = registry
            .parse(r#"LN "/data/source;1" /data/alias"#)
            .expect("parse link alias");
        let call = program.stage(0).expect("link command");

        assert_eq!(call.route.raw(), LINK_ROUTE);
        assert_eq!(text(call.get("SOURCE")).as_str(), "/data/source;1");
        assert_eq!(text(call.get("TARGET")).as_str(), "/data/alias");
    }

    #[test]
    fn parses_show_links_alias_and_rejects_bad_arity() {
        let registry = registry();
        let program = registry
            .parse("LINKS /data/alias")
            .expect("parse links alias");
        assert_eq!(program.stage(0).unwrap().route.raw(), SHOW_LINKS_ROUTE);
        assert_eq!(
            text(program.stage(0).unwrap().get("PATH")).as_str(),
            "/data/alias"
        );
        assert_eq!(
            registry.parse("LINK /data/source"),
            Err(Error::MissingArgument)
        );
        assert_eq!(
            registry.parse("LINK /data/source /data/alias /data/extra"),
            Err(Error::TooManyArguments)
        );
    }

    #[test]
    fn parses_editor_aliases_qualifiers_and_pipelines() {
        let registry = registry();
        let edit = registry
            .parse(r#"EDT "/data/notes;7" | TYPE /data/notes;7 /BINARY &"#)
            .expect("parse editor pipeline");

        assert_eq!(edit.stage_count(), 2);
        assert!(edit.background);
        assert_eq!(
            edit.stage(0).unwrap().route.raw(),
            crate::filesystem::EDIT_ROUTE
        );
        assert_eq!(
            text(edit.stage(0).unwrap().get("PATH")).as_str(),
            "/data/notes;7"
        );
        assert_eq!(
            text(edit.stage(1).unwrap().get("PATH")).as_str(),
            "/data/notes;7"
        );
        assert_eq!(
            edit.stage(1).unwrap().get("BINARY"),
            Some(Value::Boolean(true))
        );
    }

    #[test]
    fn rejects_malformed_editor_input() {
        let registry = registry();
        assert_eq!(registry.parse("EDIT"), Err(Error::MissingArgument));
        assert_eq!(
            registry.parse("EDIT /data/note /UNKNOWN"),
            Err(Error::UnknownArgument)
        );
        assert_eq!(
            registry.parse("EDIT \"/data/note"),
            Err(Error::UnterminatedQuote)
        );
        assert_eq!(
            registry.parse("EDIT /data/note |"),
            Err(Error::InvalidSyntax)
        );
    }

    #[test]
    fn parses_the_complete_filesystem_command_surface() {
        let registry = registry();
        let cases = [
            (
                "DIRECTORY \"relative folder\" /CREATE /RECURSIVE",
                DIRECTORY_ROUTE,
            ),
            ("LS /data /CONTINUATION=32", DIRECTORY_ROUTE),
            ("MKDIR /data/new /NORECURSIVE", MKDIR_ROUTE),
            ("RMDIR /data/old", RMDIR_ROUTE),
            ("RD /data/old", RMDIR_ROUTE),
            ("CREATE \"relative file\"", CREATE_FILE_ROUTE),
            ("TYPE \"relative file\" /BINARY", TYPE_ROUTE),
            ("SET DEFAULT \"/data/work dir\"", SET_DEFAULT_ROUTE),
            ("CD /data", SET_DEFAULT_ROUTE),
            ("SHOW DEFAULT", SHOW_DEFAULT_ROUTE),
            ("PWD", SHOW_DEFAULT_ROUTE),
            ("LINK /data/source /data/alias", LINK_ROUTE),
            ("LN /data/source /data/alias", LINK_ROUTE),
            ("SHOW LINKS /data/alias", SHOW_LINKS_ROUTE),
            ("LINKS /data/alias", SHOW_LINKS_ROUTE),
            ("DELETE /data/file;3", crate::filesystem::DELETE_ROUTE),
            ("DEL /data/file", crate::filesystem::DELETE_ROUTE),
            ("ERASE /data/file", crate::filesystem::DELETE_ROUTE),
            ("RM /data/file", crate::filesystem::DELETE_ROUTE),
            ("EDIT /data/file", EDIT_ROUTE),
            ("EDT /data/file;2", EDIT_ROUTE),
        ];

        for (line, route) in cases {
            assert_eq!(
                registry.parse(line).unwrap().stage(0).unwrap().route.raw(),
                route,
                "{line}"
            );
        }

        let directory = registry
            .parse("DIRECTORY \"relative folder\" /CREATE /RECURSIVE")
            .unwrap()
            .stage(0)
            .unwrap();
        assert_eq!(text(directory.get("PATH")).as_str(), "relative folder");
        assert_eq!(directory.get("CREATE"), Some(Value::Boolean(true)));
        assert_eq!(directory.get("RECURSIVE"), Some(Value::Boolean(true)));

        let listing = registry
            .parse("LS /data /CONTINUATION=32")
            .unwrap()
            .stage(0)
            .unwrap();
        assert_eq!(listing.get("CONTINUATION"), Some(Value::Integer(32)));

        assert_eq!(
            registry.parse("DIRECTORY /data /CREATE extra"),
            Err(Error::TooManyArguments)
        );
        assert_eq!(
            registry.parse("DIRECTORY /data /CREATE /CREATE"),
            Err(Error::InvalidValue)
        );
        assert_eq!(
            registry.parse("TYPE /data/file /BINARY=maybe"),
            Err(Error::InvalidValue)
        );
        assert_eq!(registry.parse("SET DEFAULT"), Err(Error::MissingArgument));
        assert_eq!(
            registry.parse("CREATE /data/file /RECURSIVE"),
            Err(Error::UnknownArgument)
        );
        assert_eq!(
            registry.parse("RMDIR /data/a /data/b"),
            Err(Error::TooManyArguments)
        );
        assert_eq!(
            registry.parse("SHOW DEFAULT /UNKNOWN"),
            Err(Error::UnknownArgument)
        );
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
        return Err(Error::InvalidValue);
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
        ArgumentKind::Integer => {
            Value::Integer(raw.parse::<i64>().map_err(|_| Error::InvalidValue)?)
        }
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
            return Ok(None);
        }
        self.skip_whitespace();
        let Some(current) = self.peek() else {
            return Ok(None);
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
                self.advance(current);
                let escaped = self.peek().ok_or(Error::InvalidSyntax)?;
                word.push_char('\\')?;
                word.push_char(escaped)?;
                self.advance(escaped);
                continue;
            }
            if let Some(expected) = quote {
                self.advance(current);
                if current == expected {
                    quote = None
                } else {
                    if matches!(current, '*' | '?' | '[' | ']' | '\\') {
                        word.push_char('\\')?;
                    }
                    word.push_char(current)?
                }
                continue;
            }
            if matches!(current, '\'' | '"') {
                quote = Some(current);
                self.advance(current);
                continue;
            }
            if current.is_ascii_whitespace() || matches!(current, '|' | '&') {
                break;
            }
            word.push_char(current)?;
            self.advance(current)
        }
        if quote.is_some() {
            return Err(Error::UnterminatedQuote);
        }
        if word.is_empty() {
            return Err(Error::InvalidSyntax);
        }
        Ok(word)
    }

    fn skip_whitespace(&mut self) {
        while let Some(current) = self.peek() {
            if !current.is_ascii_whitespace() {
                break;
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
