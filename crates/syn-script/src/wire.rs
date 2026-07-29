use syn_shell::{
    MAX_TOKEN_BYTES, Text,
    parser::{Argument, CommandCall, RouteId, Value},
};
use synos_ipc::{
    ArchiveError, Envelope, PROTOCOL_VERSION, SharedBuffer, SharedRegionId, checksum,
    structured_envelope, validate_structured,
};
use synos_status::Status;
use synos_system_model::{
    LogicalName,
    command::{
        CommandError, MAX_COMMAND_ARGUMENTS, MAX_OUTPUT_FIELDS, OutputText, OutputValue,
        StructuredOutput,
    },
};

use crate::Error;

pub const COMMAND_REQUEST_SCHEMA: u64 = 0x5359_4e53_4352_4551;
pub const COMMAND_RESPONSE_SCHEMA: u64 = 0x5359_4e53_4352_4553;

const REQUEST_MAGIC: &[u8; 4] = b"SYRQ";
const RESPONSE_MAGIC: &[u8; 4] = b"SYRS";

pub struct StructuredRequest {
    pub route: RouteId,
    pub command: LogicalName,
    pub delegated_capability: Option<u64>,
    arguments: [Option<Argument>; MAX_COMMAND_ARGUMENTS],
    pub pipeline_input: Option<StructuredOutput>,
}

impl StructuredRequest {
    pub fn arguments(&self) -> impl Iterator<Item = Argument> + '_ {
        self.arguments.iter().flatten().copied()
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.arguments()
            .find(|argument| argument.name.as_str().eq_ignore_ascii_case(name))
            .map(|argument| argument.value)
    }
}

/// Encode a typed command call into a capability-mapped IPC region.
///
/// Only a `SharedBuffer` descriptor enters the ring. Command arguments and
/// prior-stage fields stay typed in the mapped binary payload.
pub fn encode_request(
    call: CommandCall,
    pipeline_input: Option<&StructuredOutput>,
    region: SharedRegionId,
    offset: u32,
    correlation: u128,
    delegated_capability: Option<u64>,
    mapping: &mut [u8],
) -> Result<Envelope, Error> {
    let start = offset as usize;
    let destination = mapping.get_mut(start..).ok_or(Error::WireBufferTooSmall)?;
    let mut writer = Writer::new(destination);
    writer.bytes(REQUEST_MAGIC)?;
    writer.u16(PROTOCOL_VERSION)?;
    writer.u16(call.route.raw())?;
    writer.short_text(call.command.as_str())?;
    writer.u8(call.arguments().count() as u8)?;
    writer.u8(u8::from(pipeline_input.is_some()))?;
    for argument in call.arguments() {
        encode_argument(&mut writer, argument)?;
    }
    if let Some(input) = pipeline_input {
        writer.u32(input.status().raw())?;
        writer.u8(input.fields().count() as u8)?;
        for field in input.fields() {
            writer.short_text(field.name.as_str())?;
            encode_output_value(&mut writer, field.value)?;
        }
    }
    let length = writer.len();
    let payload = &mapping[start..start + length];
    Ok(structured_envelope(
        COMMAND_REQUEST_SCHEMA,
        correlation,
        SharedBuffer {
            region,
            offset,
            length: u32::try_from(length).map_err(|_| Error::WireBufferTooSmall)?,
            writable: false,
        },
        delegated_capability,
        checksum(payload),
    ))
}

pub fn decode_request(
    envelope: Envelope,
    region: SharedRegionId,
    mapping: &[u8],
) -> Result<StructuredRequest, Error> {
    let payload = resolve_payload(envelope, COMMAND_REQUEST_SCHEMA, region, mapping)?;
    let mut reader = Reader::new(payload);
    if reader.bytes(4)? != REQUEST_MAGIC {
        return Err(Error::WireCorrupt);
    }
    if reader.u16()? != PROTOCOL_VERSION {
        return Err(Error::WireCorrupt);
    }
    let route = RouteId::new(reader.u16()?).ok_or(Error::WireCorrupt)?;
    let command = LogicalName::new(reader.short_text()?).map_err(|_| Error::WireCorrupt)?;
    let argument_count = reader.u8()? as usize;
    let has_input = match reader.u8()? {
        0 => false,
        1 => true,
        _ => return Err(Error::WireCorrupt),
    };
    if argument_count > MAX_COMMAND_ARGUMENTS {
        return Err(Error::WireCorrupt);
    }
    let mut arguments: [Option<Argument>; MAX_COMMAND_ARGUMENTS] = [None; MAX_COMMAND_ARGUMENTS];
    for index in 0..argument_count {
        let argument = decode_argument(&mut reader)?;
        if arguments[..index].iter().flatten().any(|existing| {
            existing
                .name
                .as_str()
                .eq_ignore_ascii_case(argument.name.as_str())
        }) {
            return Err(Error::WireCorrupt);
        }
        arguments[index] = Some(argument);
    }
    let pipeline_input = if has_input {
        let status = Status::from_raw(reader.u32()?).ok_or(Error::WireCorrupt)?;
        let field_count = reader.u8()? as usize;
        if field_count > MAX_OUTPUT_FIELDS {
            return Err(Error::WireCorrupt);
        }
        let mut output = StructuredOutput::new(status);
        for _ in 0..field_count {
            let name = reader.short_text()?;
            if output
                .fields()
                .any(|field| field.name.as_str().eq_ignore_ascii_case(name))
            {
                return Err(Error::WireCorrupt);
            }
            let value = decode_output_value(&mut reader)?;
            output.insert(name, value).map_err(map_command_error)?;
        }
        Some(output)
    } else {
        None
    };
    if !reader.is_empty() {
        return Err(Error::WireCorrupt);
    }
    Ok(StructuredRequest {
        route,
        command,
        delegated_capability: (envelope.words[2] != 0).then_some(envelope.words[2]),
        arguments,
        pipeline_input,
    })
}

pub fn encode_response(
    output: &StructuredOutput,
    region: SharedRegionId,
    offset: u32,
    correlation: u128,
    mapping: &mut [u8],
) -> Result<Envelope, Error> {
    let start = offset as usize;
    let destination = mapping.get_mut(start..).ok_or(Error::WireBufferTooSmall)?;
    let mut writer = Writer::new(destination);
    writer.bytes(RESPONSE_MAGIC)?;
    writer.u16(PROTOCOL_VERSION)?;
    writer.u32(output.status().raw())?;
    writer.u8(output.fields().count() as u8)?;
    for field in output.fields() {
        writer.short_text(field.name.as_str())?;
        encode_output_value(&mut writer, field.value)?;
    }
    let length = writer.len();
    let payload = &mapping[start..start + length];
    Ok(structured_envelope(
        COMMAND_RESPONSE_SCHEMA,
        correlation,
        SharedBuffer {
            region,
            offset,
            length: u32::try_from(length).map_err(|_| Error::WireBufferTooSmall)?,
            writable: false,
        },
        None,
        checksum(payload),
    ))
}

pub fn decode_response(
    envelope: Envelope,
    region: SharedRegionId,
    mapping: &[u8],
) -> Result<StructuredOutput, Error> {
    let payload = resolve_payload(envelope, COMMAND_RESPONSE_SCHEMA, region, mapping)?;
    let mut reader = Reader::new(payload);
    if reader.bytes(4)? != RESPONSE_MAGIC {
        return Err(Error::WireCorrupt);
    }
    if reader.u16()? != PROTOCOL_VERSION {
        return Err(Error::WireCorrupt);
    }
    let status = Status::from_raw(reader.u32()?).ok_or(Error::WireCorrupt)?;
    let field_count = reader.u8()? as usize;
    if field_count > MAX_OUTPUT_FIELDS {
        return Err(Error::WireCorrupt);
    }
    let mut output = StructuredOutput::new(status);
    for _ in 0..field_count {
        let name = reader.short_text()?;
        if output
            .fields()
            .any(|field| field.name.as_str().eq_ignore_ascii_case(name))
        {
            return Err(Error::WireCorrupt);
        }
        let value = decode_output_value(&mut reader)?;
        output.insert(name, value).map_err(map_command_error)?;
    }
    if !reader.is_empty() {
        return Err(Error::WireCorrupt);
    }
    Ok(output)
}

fn encode_argument(writer: &mut Writer<'_>, argument: Argument) -> Result<(), Error> {
    writer.short_text(argument.name.as_str())?;
    match argument.value {
        Value::Boolean(value) => {
            writer.u8(1)?;
            writer.u8(u8::from(value))
        }
        Value::Integer(value) => {
            writer.u8(2)?;
            writer.i64(value)
        }
        Value::Text(value) => {
            writer.u8(3)?;
            writer.long_text(value.as_str())
        }
    }
}

fn decode_argument(reader: &mut Reader<'_>) -> Result<Argument, Error> {
    let name = LogicalName::new(reader.short_text()?).map_err(|_| Error::WireCorrupt)?;
    let value = match reader.u8()? {
        1 => Value::Boolean(match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::WireCorrupt),
        }),
        2 => Value::Integer(reader.i64()?),
        3 => Value::Text(
            Text::<MAX_TOKEN_BYTES>::new(reader.long_text()?).map_err(|_| Error::WireCorrupt)?,
        ),
        _ => return Err(Error::WireCorrupt),
    };
    Ok(Argument { name, value })
}

fn encode_output_value(writer: &mut Writer<'_>, value: OutputValue) -> Result<(), Error> {
    match value {
        OutputValue::Boolean(value) => {
            writer.u8(1)?;
            writer.u8(u8::from(value))
        }
        OutputValue::Integer(value) => {
            writer.u8(2)?;
            writer.i64(value)
        }
        OutputValue::Status(value) => {
            writer.u8(3)?;
            writer.u32(value.raw())
        }
        OutputValue::Text(value) => {
            writer.u8(4)?;
            writer.long_text(value.as_str())
        }
        OutputValue::Unsigned(value) => {
            writer.u8(5)?;
            writer.u64(value)
        }
    }
}

fn decode_output_value(reader: &mut Reader<'_>) -> Result<OutputValue, Error> {
    match reader.u8()? {
        1 => Ok(OutputValue::Boolean(match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::WireCorrupt),
        })),
        2 => Ok(OutputValue::Integer(reader.i64()?)),
        3 => Ok(OutputValue::Status(
            Status::from_raw(reader.u32()?).ok_or(Error::WireCorrupt)?,
        )),
        4 => Ok(OutputValue::Text(
            OutputText::new(reader.long_text()?).map_err(map_command_error)?,
        )),
        5 => Ok(OutputValue::Unsigned(reader.u64()?)),
        _ => Err(Error::WireCorrupt),
    }
}

fn resolve_payload<'a>(
    envelope: Envelope,
    schema: u64,
    region: SharedRegionId,
    mapping: &'a [u8],
) -> Result<&'a [u8], Error> {
    let descriptor = validate_structured(envelope, schema).map_err(map_archive_error)?;
    if descriptor.region != region || descriptor.writable {
        return Err(Error::WireCorrupt);
    }
    let start = descriptor.offset as usize;
    let end = start
        .checked_add(descriptor.length as usize)
        .ok_or(Error::WireCorrupt)?;
    let payload = mapping.get(start..end).ok_or(Error::WireCorrupt)?;
    if checksum(payload) != envelope.words[1] {
        return Err(Error::WireCorrupt);
    }
    Ok(payload)
}

fn map_archive_error(error: ArchiveError) -> Error {
    match error {
        ArchiveError::SchemaMismatch => Error::WireSchemaMismatch,
        ArchiveError::BufferTooSmall => Error::WireBufferTooSmall,
        ArchiveError::InvalidDescriptor | ArchiveError::InvalidLayout => Error::WireCorrupt,
    }
}

fn map_command_error(error: CommandError) -> Error {
    match error {
        CommandError::InvalidName => Error::InvalidName,
        CommandError::InvalidValue => Error::InvalidValue,
        _ => Error::WireCorrupt,
    }
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    cursor: usize,
}

impl<'a> Writer<'a> {
    const fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    const fn len(&self) -> usize {
        self.cursor
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), Error> {
        let end = self
            .cursor
            .checked_add(value.len())
            .ok_or(Error::WireBufferTooSmall)?;
        self.bytes
            .get_mut(self.cursor..end)
            .ok_or(Error::WireBufferTooSmall)?
            .copy_from_slice(value);
        self.cursor = end;
        Ok(())
    }

    fn u8(&mut self, value: u8) -> Result<(), Error> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), Error> {
        self.bytes(&value.to_be_bytes())
    }

    fn u32(&mut self, value: u32) -> Result<(), Error> {
        self.bytes(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<(), Error> {
        self.bytes(&value.to_be_bytes())
    }

    fn i64(&mut self, value: i64) -> Result<(), Error> {
        self.bytes(&value.to_be_bytes())
    }

    fn short_text(&mut self, value: &str) -> Result<(), Error> {
        self.u8(u8::try_from(value.len()).map_err(|_| Error::WireBufferTooSmall)?)?;
        self.bytes(value.as_bytes())
    }

    fn long_text(&mut self, value: &str) -> Result<(), Error> {
        self.u16(u16::try_from(value.len()).map_err(|_| Error::WireBufferTooSmall)?)?;
        self.bytes(value.as_bytes())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.cursor.checked_add(length).ok_or(Error::WireCorrupt)?;
        let value = self.bytes.get(self.cursor..end).ok_or(Error::WireCorrupt)?;
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        let bytes = self.bytes(8)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn i64(&mut self) -> Result<i64, Error> {
        let bytes = self.bytes(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn short_text(&mut self) -> Result<&'a str, Error> {
        let length = self.u8()? as usize;
        core::str::from_utf8(self.bytes(length)?).map_err(|_| Error::WireCorrupt)
    }

    fn long_text(&mut self) -> Result<&'a str, Error> {
        let length = self.u16()? as usize;
        core::str::from_utf8(self.bytes(length)?).map_err(|_| Error::WireCorrupt)
    }
}
