use syn_shell::render::{OutputFormat, render};
use synos_status::Status;
use synos_system_model::command::{OutputText, OutputValue, StructuredOutput};

#[test]
fn structured_output_json_is_a_stable_schema_with_escaped_values() {
    let mut output = StructuredOutput::new(Status::NORMAL);
    output.insert("operation", OutputValue::Text(OutputText::new("audit\nready").unwrap())).unwrap();
    output.insert("count", OutputValue::Unsigned(3)).unwrap();
    output.insert("enabled", OutputValue::Boolean(true)).unwrap();
    let rendered = render(&output, OutputFormat::Json).unwrap();
    assert_eq!(rendered.as_str(), format!("{{\"status\":{},\"fields\":{{\"operation\":\"audit\\nready\",\"count\":3,\"enabled\":true}}}}", Status::NORMAL.raw()));
}

#[test]
fn list_output_keeps_error_status_and_known_metadata_labels() {
    let mut output = StructuredOutput::new(Status::INVALID_ARGUMENT);
    output.insert("operation", OutputValue::Text(OutputText::new("read").unwrap())).unwrap();
    output.insert("path", OutputValue::Text(OutputText::new("SYS$LOG:BOOT").unwrap())).unwrap();
    output.insert("type", OutputValue::Text(OutputText::new("FILE").unwrap())).unwrap();
    output.insert("size", OutputValue::Unsigned(12)).unwrap();
    output.insert("version", OutputValue::Unsigned(4)).unwrap();
    let rendered = render(&output, OutputFormat::List).unwrap();
    assert_eq!(rendered.as_str(), format!("ERROR: status={}\nOperation: read\nPath: SYS$LOG:BOOT\nType: FILE\nSize: 12\nVersion: 4\n", Status::INVALID_ARGUMENT.raw()));
}
