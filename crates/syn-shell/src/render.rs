use core::fmt::Write;

use synos_status::Severity;
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
    if is_directory_output(output) {
        return render_directory(output)
    }
    if is_default_directory_output(output) {
        return render_default_directory(output)
    }

    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    for field in output.fields() {
        write!(&mut rendered, "{}=", field.name.as_str())
            .map_err(|_| Error::Capacity)?;
        write_value(&mut rendered, field.value, false)?;
        rendered.push_str("\n")?
    }
    Ok(rendered)
}

fn render_error_status(
    output: &StructuredOutput,
    rendered: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
) -> Result<(), Error> {
    if matches!(output.status().severity(), Severity::Error | Severity::Fatal) {
        writeln!(rendered, "ERROR: status={}", output.status().raw())
            .map_err(|_| Error::Capacity)?;
    }
    Ok(())
}

fn is_directory_output(output: &StructuredOutput) -> bool {
    output
        .fields()
        .any(|field| field.name.as_str() == "entry-count")
}

fn is_default_directory_output(output: &StructuredOutput) -> bool {
    output
        .fields()
        .any(|field| field.name.as_str() == "default-directory")
}

fn render_default_directory(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    rendered.push_str("Current directory: ")?;
    if let Some(directory) = find_value(output, "default-directory") {
        write_value(&mut rendered, directory, false)?;
    }
    rendered.push_str("\n")?;
    Ok(rendered)
}

fn render_directory(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    const ENTRY_FIELDS: [[&str; 5]; 6] = [
        [
            "entry-0-name",
            "entry-0-type",
            "entry-0-size",
            "entry-0-version",
            "entry-0-link-count",
        ],
        [
            "entry-1-name",
            "entry-1-type",
            "entry-1-size",
            "entry-1-version",
            "entry-1-link-count",
        ],
        [
            "entry-2-name",
            "entry-2-type",
            "entry-2-size",
            "entry-2-version",
            "entry-2-link-count",
        ],
        [
            "entry-3-name",
            "entry-3-type",
            "entry-3-size",
            "entry-3-version",
            "entry-3-link-count",
        ],
        [
            "entry-4-name",
            "entry-4-type",
            "entry-4-size",
            "entry-4-version",
            "entry-4-link-count",
        ],
        [
            "entry-5-name",
            "entry-5-type",
            "entry-5-size",
            "entry-5-version",
            "entry-5-link-count",
        ],
    ];

    let mut name_width = 4;
    let mut rendered_entries = 0;
    for fields in ENTRY_FIELDS {
        if let Some(OutputValue::Text(name)) = find_value(output, fields[0]) {
            name_width = name_width.max(name.as_str().len());
        }
    }

    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    rendered.push_str("Directory: ")?;
    if let Some(path) = find_value(output, "path") {
        write_value(&mut rendered, path, false)?;
    }
    rendered.push_str("\nEntries: ")?;
    if let Some(count) = find_value(output, "entry-count") {
        write_value(&mut rendered, count, false)?;
    }
    rendered.push_str("\n\n")?;

    write_table_text(&mut rendered, "NAME", name_width, false)?;
    rendered.push_str("  ")?;
    write_table_text(&mut rendered, "TYPE", 10, false)?;
    rendered.push_str("  ")?;
    write_table_text(&mut rendered, "SIZE", 8, true)?;
    rendered.push_str("  ")?;
    write_table_text(&mut rendered, "VERSION", 7, true)?;
    rendered.push_str("  ")?;
    write_table_text(&mut rendered, "LINKS", 5, true)?;
    rendered.push_str("\n")?;

    for fields in ENTRY_FIELDS {
        let Some(OutputValue::Text(name)) = find_value(output, fields[0]) else {
            continue
        };
        let Some(file_type) = find_value(output, fields[1]) else {
            continue
        };
        let Some(size) = find_value(output, fields[2]) else {
            continue
        };
        let Some(version) = find_value(output, fields[3]) else {
            continue
        };
        let Some(link_count) = find_value(output, fields[4]) else {
            continue
        };

        write_table_text(&mut rendered, name.as_str(), name_width, false)?;
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, file_type, 10, false)?;
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, size, 8, true)?;
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, version, 7, true)?;
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, link_count, 5, true)?;
        rendered.push_str("\n")?;
        rendered_entries += 1;
    }

    if rendered_entries == 0 {
        rendered.push_str("(empty)\n")?;
    }

    if let Some(next) = find_value(output, "next") {
        rendered.push_str("\nMore entries available (continuation: ")?;
        write_value(&mut rendered, next, false)?;
        rendered.push_str(")\n")?;
    }

    Ok(rendered)
}

fn find_value(output: &StructuredOutput, name: &str) -> Option<OutputValue> {
    output
        .fields()
        .find(|field| field.name.as_str() == name)
        .map(|field| field.value)
}

fn write_table_text(
    output: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    value: &str,
    width: usize,
    right_aligned: bool,
) -> Result<(), Error> {
    let padding = width.saturating_sub(value.len());
    if right_aligned {
        write_spaces(output, padding)?;
    }
    output.push_str(value)?;
    if !right_aligned {
        write_spaces(output, padding)?;
    }
    Ok(())
}

fn write_table_value(
    output: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    value: OutputValue,
    width: usize,
    right_aligned: bool,
) -> Result<(), Error> {
    let mut rendered = Text::<64>::empty();
    write_value(&mut rendered, value, false)?;
    write_table_text(output, rendered.as_str(), width, right_aligned)
}

fn write_spaces(
    output: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    count: usize,
) -> Result<(), Error> {
    for _ in 0..count {
        output.push_str(" ")?;
    }
    Ok(())
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

fn write_value<const CAPACITY: usize>(
    output: &mut Text<CAPACITY>,
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

fn write_escaped<const CAPACITY: usize>(
    output: &mut Text<CAPACITY>,
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
