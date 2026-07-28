use core::fmt::Write;

use synos_system_model::command::{OutputValue, StructuredOutput};

use crate::{Error, Text};

pub const MAX_RENDERED_OUTPUT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    List,
    Json,
}

pub fn render(
    output: &StructuredOutput,
    format: OutputFormat,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    match format {
        OutputFormat::List => render_list(output),
        OutputFormat::Json => render_json(output),
    }
}

fn render_list(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    writeln!(&mut rendered, "$STATUS={}", output.status().raw())
        .map_err(|_| Error::Capacity)?;
    for field in output.fields() {
        write!(&mut rendered, "{}=", field.name.as_str())
            .map_err(|_| Error::Capacity)?;
        write_value(&mut rendered, field.value, false)?;
        rendered.push_str("\n")?
    }
    Ok(rendered)
}

fn render_json(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    write!(
        &mut rendered,
        "{{\"status\":{},\"fields\":{{",
        output.status().raw()
    )
    .map_err(|_| Error::Capacity)?;
    for (index, field) in output.fields().enumerate() {
        if index != 0 {
            rendered.push_str(",")?
        }
        rendered.push_str("\"")?;
        write_escaped(&mut rendered, field.name.as_str())?;
        rendered.push_str("\":")?;
        write_value(&mut rendered, field.value, true)?
    }
    rendered.push_str("}}")?;
    Ok(rendered)
}

fn write_value(
    output: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    value: OutputValue,
    json: bool,
) -> Result<(), Error> {
    match value {
        OutputValue::Boolean(value) => {
            output.push_str(if value { "true" } else { "false" })
        }
        OutputValue::Integer(value) => {
            write!(output, "{value}").map_err(|_| Error::Capacity)
        }
        OutputValue::Unsigned(value) => {
            write!(output, "{value}").map_err(|_| Error::Capacity)
        }
        OutputValue::Status(value) => {
            write!(output, "{}", value.raw()).map_err(|_| Error::Capacity)
        }
        OutputValue::Text(value) => {
            if json {
                output.push_str("\"")?;
                write_escaped(output, value.as_str())?;
                output.push_str("\"")
            } else {
                output.push_str(value.as_str())
            }
        }
    }
}

fn write_escaped(
    output: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    value: &str,
) -> Result<(), Error> {
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\"")?,
            '\\' => output.push_str("\\\\")?,
            '\n' => output.push_str("\\n")?,
            '\r' => output.push_str("\\r")?,
            '\t' => output.push_str("\\t")?,
            character if character.is_control() => {
                write!(output, "\\u{:04x}", character as u32)
                    .map_err(|_| Error::Capacity)?
            }
            character => output.push_char(character)?,
        }
    }
    Ok(())
}
