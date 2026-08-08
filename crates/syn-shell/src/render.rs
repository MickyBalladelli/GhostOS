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
    if is_created_output(output) {
        return render_created(output)
    }
    if is_linked_output(output) {
        return render_linked(output)
    }
    if is_deleted_output(output) {
        return render_deleted(output)
    }
    if is_removed_output(output) {
        return render_removed(output)
    }
    if is_show_interfaces_output(output) {
        return render_interfaces(output)
    }
    if is_set_interface_output(output) {
        return render_set_interface(output)
    }
    if is_metadata_output(output) {
        return render_metadata(output)
    }
    if is_uptime_output(output) {
        return render_uptime(output)
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
        writeln!(
            rendered,
            "ERROR: status={} ({})",
            output.status().raw(),
            output.status().message()
        )
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

fn is_metadata_output(output: &StructuredOutput) -> bool {
    output
        .fields()
        .any(|field| field.name.as_str() == "operation")
}

fn is_uptime_output(output: &StructuredOutput) -> bool {
    output.fields().any(|field| field.name.as_str() == "uptime-us")
}

fn is_show_interfaces_output(output: &StructuredOutput) -> bool {
    matches!(
        find_value(output, "operation"),
        Some(OutputValue::Text(value))
            if value.as_str() == "show-network"
                || value.as_str() == "show-interfaces"
                || value.as_str() == "show-interface"
    )
}

const NETWORK_ROUTE_FIELD_NAMES: [[&str; 4]; 4] = [
    [
        "route1-destination",
        "route1-gateway",
        "route1-interface",
        "route1-metric",
    ],
    [
        "route2-destination",
        "route2-gateway",
        "route2-interface",
        "route2-metric",
    ],
    [
        "route3-destination",
        "route3-gateway",
        "route3-interface",
        "route3-metric",
    ],
    [
        "route4-destination",
        "route4-gateway",
        "route4-interface",
        "route4-metric",
    ],
];

fn is_set_interface_output(output: &StructuredOutput) -> bool {
    matches!(
        find_value(output, "operation"),
        Some(OutputValue::Text(value)) if value.as_str() == "set-interface"
    )
}

const INTERFACE_FIELD_NAMES: [[&str; 7]; 4] = [
    [
        "interface1-name",
        "interface1-address",
        "interface1-gateway",
        "interface1-mtu",
        "interface1-enabled",
        "interface1-link-up",
        "interface1-mode",
    ],
    [
        "interface2-name",
        "interface2-address",
        "interface2-gateway",
        "interface2-mtu",
        "interface2-enabled",
        "interface2-link-up",
        "interface2-mode",
    ],
    [
        "interface3-name",
        "interface3-address",
        "interface3-gateway",
        "interface3-mtu",
        "interface3-enabled",
        "interface3-link-up",
        "interface3-mode",
    ],
    [
        "interface4-name",
        "interface4-address",
        "interface4-gateway",
        "interface4-mtu",
        "interface4-enabled",
        "interface4-link-up",
        "interface4-mode",
    ],
];

const INTERFACE_DHCP_FIELD_NAMES: [[&str; 5]; 4] = [
    [
        "interface1-dhcp-state",
        "interface1-dhcp-server",
        "interface1-dhcp-expires-ms",
        "interface1-dns0",
        "interface1-dns1",
    ],
    [
        "interface2-dhcp-state",
        "interface2-dhcp-server",
        "interface2-dhcp-expires-ms",
        "interface2-dns0",
        "interface2-dns1",
    ],
    [
        "interface3-dhcp-state",
        "interface3-dhcp-server",
        "interface3-dhcp-expires-ms",
        "interface3-dns0",
        "interface3-dns1",
    ],
    [
        "interface4-dhcp-state",
        "interface4-dhcp-server",
        "interface4-dhcp-expires-ms",
        "interface4-dns0",
        "interface4-dns1",
    ],
];

#[inline(never)]
fn render_interfaces(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    let singular = matches!(
        find_value(output, "operation"),
        Some(OutputValue::Text(value)) if value.as_str() == "show-interface"
    );
    let network = matches!(
        find_value(output, "operation"),
        Some(OutputValue::Text(value)) if value.as_str() == "show-network"
    );
    if network {
        rendered.push_str("Network: ")?;
        if let Some(hostname) = find_value(output, "hostname") {
            write_value(&mut rendered, hostname, false)?;
        }
    } else if singular {
        rendered.push_str("Interface: ")?;
        if let Some(name) = find_value(output, "interface1-name") {
            write_value(&mut rendered, name, false)?;
        }
    } else {
        rendered.push_str("Interfaces: ")?;
        if let Some(count) = find_value(output, "interface-count") {
            write_value(&mut rendered, count, false)?;
        }
    }
    rendered.push_str(" (generation ")?;
    if let Some(generation) = find_value(output, "generation") {
        write_value(&mut rendered, generation, false)?;
    }
    rendered.push_str(")\n")?;
    if network {
        rendered.push_str("Interfaces: ")?;
        if let Some(count) = find_value(output, "interface-count") {
            write_value(&mut rendered, count, false)?;
        }
        rendered.push_str("\n")?;
    }
    let mut rows = 0;
    for (index, names) in INTERFACE_FIELD_NAMES.iter().enumerate() {
        let Some(name) = find_value(output, names[0]) else {
            continue
        };
        render_labeled_value(&mut rendered, "Interface", name)?;
        render_interface_field(&mut rendered, output, names[1], "  Address")?;
        render_interface_field(&mut rendered, output, names[2], "  Gateway")?;
        render_interface_field(&mut rendered, output, names[6], "  Mode")?;
        render_interface_field(&mut rendered, output, names[3], "  MTU")?;
        render_interface_field(&mut rendered, output, names[4], "  Enabled")?;
        render_interface_field(&mut rendered, output, names[5], "  Link up")?;

        if let Some(state) = find_value(output, INTERFACE_DHCP_FIELD_NAMES[index][0]) {
            render_labeled_value(&mut rendered, "  DHCP state", state)?;
            render_interface_field(
                &mut rendered,
                output,
                INTERFACE_DHCP_FIELD_NAMES[index][1],
                "  DHCP server",
            )?;
            render_interface_field(
                &mut rendered,
                output,
                INTERFACE_DHCP_FIELD_NAMES[index][2],
                "  DHCP expires (ms)",
            )?;
            render_interface_field(
                &mut rendered,
                output,
                INTERFACE_DHCP_FIELD_NAMES[index][3],
                "  DNS 1",
            )?;
            render_interface_field(
                &mut rendered,
                output,
                INTERFACE_DHCP_FIELD_NAMES[index][4],
                "  DNS 2",
            )?;
        }
        rendered.push_str("\n")?;
        rows += 1;
    }

    if rows == 0 {
        rendered.push_str("(none)\n")?;
    }
    if let Some(next) = find_value(output, "next-interface") {
        rendered.push_str("\nMore interfaces available (continuation: ")?;
        write_value(&mut rendered, next, false)?;
        rendered.push_str(")\n")?;
    }
    if network {
        rendered.push_str("Routes: ")?;
        if let Some(count) = find_value(output, "route-count") {
            write_value(&mut rendered, count, false)?;
        }
        rendered.push_str("\n")?;
        for names in NETWORK_ROUTE_FIELD_NAMES {
            let Some(destination) = find_value(output, names[0]) else {
                continue
            };
            rendered.push_str("  Route: ")?;
            write_value(&mut rendered, destination, false)?;
            render_interface_field(&mut rendered, output, names[1], "    Gateway")?;
            render_interface_field(&mut rendered, output, names[2], "    Interface")?;
            render_interface_field(&mut rendered, output, names[3], "    Metric")?;
        }
        if let Some(next) = find_value(output, "next-route") {
            rendered.push_str("  More routes available (continuation: ")?;
            write_value(&mut rendered, next, false)?;
            rendered.push_str(")\n")?;
        }
    }
    Ok(rendered)
}

fn render_interface_field(
    rendered: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    output: &StructuredOutput,
    field: &str,
    label: &str,
) -> Result<(), Error> {
    if let Some(value) = find_value(output, field) {
        render_labeled_value(rendered, label, value)?;
    }
    Ok(())
}

#[inline(never)]
fn render_set_interface(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    rendered.push_str("Interface updated\n")?;
    for (field, label) in [
        ("interface", "  Name"),
        ("mode", "  Mode"),
        ("address", "  Address"),
        ("gateway", "  Gateway"),
        ("mtu", "  MTU"),
        ("enabled", "  Enabled"),
        ("link-up", "  Link up"),
        ("dhcp-state", "  DHCP state"),
        ("dhcp-server", "  DHCP server"),
        ("dhcp-expires-ms", "  DHCP expires (ms)"),
        ("dns0", "  DNS 1"),
        ("dns1", "  DNS 2"),
        ("generation", "  Generation"),
    ] {
        render_interface_field(&mut rendered, output, field, label)?;
    }
    Ok(rendered)
}

fn render_uptime(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    let days = find_value(output, "days").and_then(unsigned_value).unwrap_or(0);
    let hours = find_value(output, "hours").and_then(unsigned_value).unwrap_or(0);
    let minutes = find_value(output, "minutes")
        .and_then(unsigned_value)
        .unwrap_or(0);
    let seconds = find_value(output, "seconds")
        .and_then(unsigned_value)
        .unwrap_or(0);
    write!(&mut rendered, "Uptime: ").map_err(|_| Error::Capacity)?;
    if days != 0 {
        write!(&mut rendered, "{days} day{}, ", if days == 1 { "" } else { "s" })
            .map_err(|_| Error::Capacity)?;
    }
    write!(&mut rendered, "{hours:02}:{minutes:02}:{seconds:02}\n")
        .map_err(|_| Error::Capacity)?;
    Ok(rendered)
}

fn is_created_output(output: &StructuredOutput) -> bool {
    matches!(find_value(output, "operation"), Some(OutputValue::Text(value)) if value.as_str() == "created")
}

fn is_linked_output(output: &StructuredOutput) -> bool {
    matches!(find_value(output, "operation"), Some(OutputValue::Text(value)) if value.as_str() == "linked")
}

fn is_deleted_output(output: &StructuredOutput) -> bool {
    matches!(find_value(output, "operation"), Some(OutputValue::Text(value)) if value.as_str() == "deleted")
}

fn is_removed_output(output: &StructuredOutput) -> bool {
    matches!(find_value(output, "operation"), Some(OutputValue::Text(value)) if value.as_str() == "removed")
}

fn render_removed(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    if let Some(path) = find_value(output, "path") {
        write_value(&mut rendered, path, false)?;
        rendered.push_str(" was deleted")?;
        rendered.push_str("\n")?;
    }
    Ok(rendered)
}

fn render_deleted(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    if let Some(path) = find_value(output, "path") {
        write_value(&mut rendered, path, false)?;
        if let Some(version) = find_value(output, "version") {
            rendered.push_str(";")?;
            write_value(&mut rendered, version, false)?;
        }
    }
    rendered.push_str(" was deleted\n")?;
    Ok(rendered)
}

fn render_created(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    if let Some(path) = find_value(output, "path") {
        write_value(&mut rendered, path, false)?;
        if !is_directory_output_type(output) {
            rendered.push_str(";")?;
            if let Some(version) = find_value(output, "version") {
                write_value(&mut rendered, version, false)?;
            }
        }
    }
    rendered.push_str(" was created\n")?;
    Ok(rendered)
}

fn render_linked(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;
    if let (Some(target), Some(source)) = (find_value(output, "path"), find_value(output, "source")) {
        write_value(&mut rendered, target, false)?;
        rendered.push_str(" is now a link to ")?;
        write_value(&mut rendered, source, false)?;
        rendered.push_str("\n")?;
    }
    Ok(rendered)
}

fn render_metadata(
    output: &StructuredOutput,
) -> Result<Text<MAX_RENDERED_OUTPUT_BYTES>, Error> {
    let mut rendered = Text::empty();
    render_error_status(output, &mut rendered)?;

    const FIELDS: [(&str, &str); 5] = [
        ("Operation", "operation"),
        ("Path", "path"),
        ("Type", "type"),
        ("Size", "size"),
        ("Version", "version"),
    ];

    for (label, field) in FIELDS {
        if field == "size" && is_directory_output_type(output) {
            continue
        }
        let Some(value) = find_value(output, field) else {
            continue
        };
        render_labeled_value(&mut rendered, label, value)?;
    }

    for field in output.fields() {
        if FIELDS.iter().any(|(_, name)| *name == field.name.as_str()) {
            continue
        }
        render_labeled_value(&mut rendered, field.name.as_str(), field.value)?;
    }

    Ok(rendered)
}

fn render_labeled_value(
    rendered: &mut Text<MAX_RENDERED_OUTPUT_BYTES>,
    label: &str,
    value: OutputValue,
) -> Result<(), Error> {
    // Prefer push_str over `write!("{label}")` — size/LTO builds have miscompiled
    // some format_args str writes in the guest kernel.
    rendered.push_str(label)?;
    rendered.push_str(": ")?;
    write_value(rendered, value, false)?;
    rendered.push_str("\n")?;
    Ok(())
}

fn is_directory_output_type(output: &StructuredOutput) -> bool {
    find_value(output, "type").is_some_and(is_directory_value)
}

fn is_directory_value(value: OutputValue) -> bool {
    matches!(value, OutputValue::Text(value) if value.as_str() == "DIRECTORY")
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
    const ENTRY_FIELDS: [[&str; 4]; 6] = [
        [
            "entry-0-name",
            "entry-0-type",
            "entry-0-size",
            "entry-0-version",
        ],
        [
            "entry-1-name",
            "entry-1-type",
            "entry-1-size",
            "entry-1-version",
        ],
        [
            "entry-2-name",
            "entry-2-type",
            "entry-2-size",
            "entry-2-version",
        ],
        [
            "entry-3-name",
            "entry-3-type",
            "entry-3-size",
            "entry-3-version",
        ],
        [
            "entry-4-name",
            "entry-4-type",
            "entry-4-size",
            "entry-4-version",
        ],
        [
            "entry-5-name",
            "entry-5-type",
            "entry-5-size",
            "entry-5-version",
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
        write_table_text(&mut rendered, name.as_str(), name_width, false)?;
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, file_type, 10, false)?;
        rendered.push_str("  ")?;
        if is_directory_value(file_type) {
            write_table_text(&mut rendered, "-", 8, true)?;
        } else {
            write_table_value(&mut rendered, size, 8, true)?;
        }
        rendered.push_str("  ")?;
        write_table_value(&mut rendered, version, 7, true)?;
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

fn unsigned_value(value: OutputValue) -> Option<u64> {
    match value {
        OutputValue::Unsigned(value) => Some(value),
        _ => None,
    }
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
        "{{\"status\":{},\"message\":\"{}\",\"fields\":{{",
        output.status().raw(),
        output.status().message()
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
