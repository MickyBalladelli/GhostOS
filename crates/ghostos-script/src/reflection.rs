use ghostos_shell::parser::{CommandRegistration, CommandRegistry};
use ghostos_system_model::command::{ArgumentKind, ArgumentSpec};

use crate::Error;

/// Deterministic OpenAI-compatible function schema reflection.
///
/// Commands and parameters keep their registration order. Command names are
/// normalized to lowercase function names with punctuation replaced by `_`.
pub struct ToolSchemaExporter;

impl ToolSchemaExporter {
    pub fn export<const COMMANDS: usize>(
        registry: &CommandRegistry<COMMANDS>,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        Self::validate_names(registry)?;
        let mut writer = JsonWriter::new(destination);
        writer.push(b"[");
        for (index, registration) in registry.registrations().enumerate() {
            if index != 0 {
                writer.push(b",")
            }
            write_command(&mut writer, registration)
        }
        writer.push(b"]");
        writer.finish()
    }

    /// Resolve a reflected function name back to its registered command.
    pub fn resolve<const COMMANDS: usize>(
        registry: &CommandRegistry<COMMANDS>,
        function_name: &str,
    ) -> Result<Option<CommandRegistration>, Error> {
        Self::validate_names(registry)?;
        Ok(registry
            .registrations()
            .find(|registration| normalized_eq(registration.spec.name.as_str(), function_name)))
    }

    fn validate_names<const COMMANDS: usize>(
        registry: &CommandRegistry<COMMANDS>,
    ) -> Result<(), Error> {
        for (index, command) in registry.registrations().enumerate() {
            if registry.registrations().take(index).any(|earlier| {
                normalized_names_equal(earlier.spec.name.as_str(), command.spec.name.as_str())
            }) {
                return Err(Error::SchemaNameCollision);
            }
        }
        Ok(())
    }
}

fn write_command(writer: &mut JsonWriter<'_>, registration: CommandRegistration) {
    writer.push(b"{\"type\":\"function\",\"function\":{\"name\":\"");
    writer.normalized(registration.spec.name.as_str());
    writer.push(b"\",\"description\":\"Execute GhostOS command ");
    writer.escaped(registration.spec.name.as_str().as_bytes());
    writer.push(b" on route ");
    writer.unsigned(u64::from(registration.route.raw()));
    writer.push(b"\",\"parameters\":{\"type\":\"object\",\"properties\":{");
    for (index, argument) in registration.spec.arguments().enumerate() {
        if index != 0 {
            writer.push(b",")
        }
        write_property(writer, argument)
    }
    writer.push(b"},\"required\":[");
    for (index, argument) in registration
        .spec
        .arguments()
        .filter(|argument| argument.required)
        .enumerate()
    {
        if index != 0 {
            writer.push(b",")
        }
        writer.push(b"\"");
        writer.escaped(argument.name.as_str().as_bytes());
        writer.push(b"\"")
    }
    writer.push(b"],\"additionalProperties\":false}}}")
}

fn write_property(writer: &mut JsonWriter<'_>, argument: ArgumentSpec) {
    writer.push(b"\"");
    writer.escaped(argument.name.as_str().as_bytes());
    writer.push(b"\":{\"type\":\"");
    writer.push(match argument.kind {
        ArgumentKind::Boolean => b"boolean",
        ArgumentKind::Integer => b"integer",
        ArgumentKind::Text => b"string",
    });
    writer.push(b"\",\"description\":\"");
    writer.push(if argument.required {
        b"Required "
    } else {
        b"Optional "
    });
    writer.push(if argument.positional {
        b"positional argument"
    } else {
        b"qualifier"
    });
    writer.push(b"\"}")
}

fn normalized_eq(command_name: &str, function_name: &str) -> bool {
    if command_name.len() != function_name.len() {
        return false;
    }
    command_name
        .bytes()
        .zip(function_name.bytes())
        .all(|(command, function)| normalize_byte(command) == function)
}

fn normalized_names_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .all(|(left, right)| normalize_byte(left) == normalize_byte(right))
}

const fn normalize_byte(byte: u8) -> u8 {
    if byte.is_ascii_alphanumeric() {
        byte.to_ascii_lowercase()
    } else {
        b'_'
    }
}

struct JsonWriter<'a> {
    destination: &'a mut [u8],
    required: usize,
}

impl<'a> JsonWriter<'a> {
    const fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            required: 0,
        }
    }

    fn push(&mut self, value: &[u8]) {
        for byte in value {
            self.byte(*byte)
        }
    }

    fn byte(&mut self, value: u8) {
        if let Some(slot) = self.destination.get_mut(self.required) {
            *slot = value
        }
        self.required = self.required.saturating_add(1)
    }

    fn normalized(&mut self, value: &str) {
        for byte in value.bytes() {
            self.byte(normalize_byte(byte))
        }
    }

    fn escaped(&mut self, value: &[u8]) {
        for byte in value {
            match byte {
                b'"' => self.push(b"\\\""),
                b'\\' => self.push(b"\\\\"),
                0x00..=0x1f => {
                    self.push(b"\\u00");
                    self.byte(hex(byte >> 4));
                    self.byte(hex(byte & 0x0f))
                }
                _ => self.byte(*byte),
            }
        }
    }

    fn unsigned(&mut self, mut value: u64) {
        let mut digits = [0_u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.push(&digits[start..])
    }

    fn finish(self) -> Result<usize, Error> {
        if self.required > self.destination.len() {
            Err(Error::SchemaBufferTooSmall {
                required: self.required,
            })
        } else {
            Ok(self.required)
        }
    }
}

const fn hex(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + value - 10,
    }
}
