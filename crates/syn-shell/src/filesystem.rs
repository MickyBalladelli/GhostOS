use synos_status::Status;
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputText, OutputValue, StructuredOutput,
};

use crate::{
    Error, Text,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_DIRECTORY_PAGE_ENTRIES: usize = 32;
pub const MAX_LINK_PAGE_ENTRIES: usize = 16;
pub const MAX_VISIBLE_DIRECTORY_ENTRIES: usize = 6;
pub const MAX_TYPE_OUTPUT_BYTES: usize = 4096;

pub const DIRECTORY_ROUTE: u16 = 32;
pub const CREATE_FILE_ROUTE: u16 = 33;
pub const TYPE_ROUTE: u16 = 34;
pub const SET_DEFAULT_ROUTE: u16 = 35;
pub const SHOW_DEFAULT_ROUTE: u16 = 36;
pub const MKDIR_ROUTE: u16 = 37;
pub const LINK_ROUTE: u16 = 38;
pub const SHOW_LINKS_ROUTE: u16 = 39;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Path {
    bytes: [u8; MAX_PATH_BYTES],
    len: u16,
}

impl Path {
    pub const ROOT: Self = Self {
        bytes: {
            let mut bytes = [0; MAX_PATH_BYTES];
            bytes[0] = b'/';
            bytes
        },
        len: 1,
    };

    pub fn new(value: &str) -> Result<Self, Status> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_PATH_BYTES
            || bytes.contains(&0)
            || bytes.contains(&b'\\')
            || (bytes.len() > 1 && bytes.ends_with(b"/"))
            || bytes.windows(2).any(|pair| pair == b"//")
        {
            return Err(Status::INVALID_ARGUMENT);
        }
        let mut path = Self {
            bytes: [0; MAX_PATH_BYTES],
            len: bytes.len() as u16,
        };
        path.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(path)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("path invariant")
    }
}

/// Split the optional OpenVMS/SynFS version selector from a path.
///
/// `;0` means the latest version. Any other numeric selector means an exact
/// version. A selector is only valid at the end of the path.
pub fn split_version_selector(path: &str) -> Result<(&str, Option<u32>), Status> {
    let Some((file, suffix)) = path.rsplit_once(';') else {
        return Ok((path, None))
    };
    if file.is_empty()
        || file.contains(';')
        || suffix.is_empty()
        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    let version = suffix
        .parse::<u32>()
        .map_err(|_| Status::INVALID_ARGUMENT)?;
    Ok((file, Some(version)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryType {
    File,
    Directory,
    Symlink,
}

impl EntryType {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "FILE",
            Self::Directory => "DIRECTORY",
            Self::Symlink => "SYMLINK",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    pub path: Path,
    pub file_type: EntryType,
    pub size: u64,
    pub version: u32,
    pub link_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: Path,
    pub file_type: EntryType,
    pub size: u64,
    pub version: u32,
    pub link_count: u32,
}

pub struct DirectoryPage {
    entries: [Option<DirectoryEntry>; MAX_DIRECTORY_PAGE_ENTRIES],
    count: usize,
    pub next: Option<u32>,
}

pub struct LinkPage {
    entries: [Option<Path>; MAX_LINK_PAGE_ENTRIES],
    count: usize,
}

impl LinkPage {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_LINK_PAGE_ENTRIES],
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.entries = [None; MAX_LINK_PAGE_ENTRIES];
        self.count = 0
    }

    pub fn push(&mut self, path: Path) -> Result<(), Status> {
        let slot = self.entries.get_mut(self.count).ok_or(Status::NO_SPACE)?;
        *slot = Some(path);
        self.count += 1;
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = Path> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }
}

impl DirectoryPage {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_DIRECTORY_PAGE_ENTRIES],
            count: 0,
            next: None,
        }
    }

    pub fn clear(&mut self) {
        self.entries = [None; MAX_DIRECTORY_PAGE_ENTRIES];
        self.count = 0;
        self.next = None
    }

    pub fn push(&mut self, entry: DirectoryEntry) -> Result<(), Status> {
        let slot = self.entries.get_mut(self.count).ok_or(Status::NO_SPACE)?;
        *slot = Some(entry);
        self.count += 1;
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = DirectoryEntry> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.count
    }
}

pub trait FileOutput {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Status>;
}

pub trait FilesystemSource {
    fn directory_exists(&mut self, path: &str) -> Result<bool, Status>;
    fn list(
        &mut self,
        path: &str,
        continuation: Option<u32>,
        output: &mut DirectoryPage,
    ) -> Result<(), Status>;
    fn create_directory(&mut self, path: &str, recursive: bool) -> Result<FileMetadata, Status>;
    fn create_file(&mut self, path: &str) -> Result<FileMetadata, Status>;
    fn link(&mut self, _source: &str, _target: &str) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }

    fn list_links(&mut self, _path: &str, _output: &mut LinkPage) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Open the selected version read-only, stream bounded reads to `output`,
    /// and close the handle before returning. Implementations must close the
    /// handle when a read or output write fails too.
    fn type_file(
        &mut self,
        path: &str,
        binary: bool,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status>;
}

#[derive(Clone, Copy)]
pub struct ShellSession {
    default_directory: Path,
}

impl ShellSession {
    pub const fn new() -> Self {
        Self {
            default_directory: Path::ROOT,
        }
    }

    pub const fn default_directory(&self) -> Path {
        self.default_directory
    }

    pub fn resolve(&self, path: Option<&str>) -> Result<Path, Status> {
        let path = path.unwrap_or(self.default_directory.as_str());
        let (path_without_version, version) = split_version_selector(path)?;
        if path_without_version.is_empty() {
            return Err(Status::INVALID_ARGUMENT);
        }
        if path_without_version != "/"
            && (path_without_version.ends_with('/')
                || path_without_version
                    .as_bytes()
                    .windows(2)
                    .any(|pair| pair == b"//"))
        {
            return Err(Status::INVALID_ARGUMENT);
        }
        if path_without_version.contains('\\') || path_without_version.contains('\0') {
            return Err(Status::INVALID_ARGUMENT);
        }
        let mut source = Text::<{ MAX_PATH_BYTES * 2 + 1 }>::empty();
        if !path_without_version.starts_with('/') {
            source
                .push_str(self.default_directory.as_str())
                .map_err(|_| Status::INVALID_ARGUMENT)?;
            if !source.as_str().ends_with('/') {
                source
                    .push_char('/')
                    .map_err(|_| Status::INVALID_ARGUMENT)?;
            }
        }
        source
            .push_str(path_without_version)
            .map_err(|_| Status::INVALID_ARGUMENT)?;
        let resolved = canonicalize(source.as_str())?;
        let mut result = Text::<MAX_PATH_BYTES>::new(resolved.as_str())
            .map_err(|_| Status::INVALID_ARGUMENT)?;
        if let Some(version) = version {
            result
                .push_char(';')
                .map_err(|_| Status::INVALID_ARGUMENT)?;
            let mut version_text = Text::<10>::empty();
            write_u32(&mut version_text, version)?;
            result
                .push_str(version_text.as_str())
                .map_err(|_| Status::INVALID_ARGUMENT)?;
        }
        Path::new(result.as_str())
    }

    pub fn set_default<S: FilesystemSource>(
        &mut self,
        source: &mut S,
        path: &str,
    ) -> Result<Path, Status> {
        if path.contains(';') {
            return Err(Status::INVALID_ARGUMENT);
        }
        let resolved = self.resolve(Some(path))?;
        if !source.directory_exists(resolved.as_str())? {
            return Err(Status::NOT_FOUND);
        }
        self.default_directory = resolved;
        Ok(resolved)
    }
}

impl Default for ShellSession {
    fn default() -> Self {
        Self::new()
    }
}

fn canonicalize(source: &str) -> Result<Path, Status> {
    let mut result = Path::ROOT;
    let mut component_starts = [0usize; MAX_PATH_BYTES / 2 + 1];
    let mut component_count = 0;
    let mut cursor = 1;

    for component in source.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            if component_count == 0 {
                return Err(Status::INVALID_ARGUMENT);
            }
            result.len = component_starts[component_count - 1] as u16;
            component_count -= 1;
            cursor = result.len as usize;
            continue;
        }
        if component.contains('\0') || component_count == component_starts.len() {
            return Err(Status::INVALID_ARGUMENT);
        }
        let start = if cursor == 1 { 1 } else { cursor + 1 };
        let end = start
            .checked_add(component.len())
            .ok_or(Status::INVALID_ARGUMENT)?;
        if end > MAX_PATH_BYTES {
            return Err(Status::INVALID_ARGUMENT);
        }
        if cursor > 1 {
            result.bytes[cursor] = b'/';
            cursor += 1;
        }
        component_starts[component_count] = cursor;
        result.bytes[cursor..cursor + component.len()].copy_from_slice(component.as_bytes());
        cursor = end;
        result.len = cursor as u16;
        component_count += 1;
    }
    Ok(result)
}

pub fn register_filesystem_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let path = ArgumentSpec::new("PATH", ArgumentKind::Text, false, true)
        .map_err(|_| Error::InvalidValue)?;
    let create = ArgumentSpec::new("CREATE", ArgumentKind::Boolean, false, false)
        .map_err(|_| Error::InvalidValue)?;
    let recursive = ArgumentSpec::new("RECURSIVE", ArgumentKind::Boolean, false, false)
        .map_err(|_| Error::InvalidValue)?;
    let continuation = ArgumentSpec::new("CONTINUATION", ArgumentKind::Integer, false, false)
        .map_err(|_| Error::InvalidValue)?;
    let required_path = ArgumentSpec::new("PATH", ArgumentKind::Text, true, true)
        .map_err(|_| Error::InvalidValue)?;
    let directory = CommandSpec::new("DIRECTORY", &[path, create, recursive, continuation])
        .map_err(|_| Error::InvalidValue)?;
    registry.register(directory, route(DIRECTORY_ROUTE))?;
    registry.register(
        CommandSpec::new("MKDIR", &[required_path, recursive]).map_err(|_| Error::InvalidValue)?,
        route(MKDIR_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("LS", &[path, continuation]).map_err(|_| Error::InvalidValue)?,
        route(DIRECTORY_ROUTE),
    )?;

    registry.register(
        CommandSpec::new("CREATE", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(CREATE_FILE_ROUTE),
    )?;
    let binary = ArgumentSpec::new("BINARY", ArgumentKind::Boolean, false, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("TYPE", &[required_path, binary]).map_err(|_| Error::InvalidValue)?,
        route(TYPE_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SET-DEFAULT", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(SET_DEFAULT_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("CD", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(SET_DEFAULT_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-DEFAULT", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_DEFAULT_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("PWD", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_DEFAULT_ROUTE),
    )?;
    let source = ArgumentSpec::new("SOURCE", ArgumentKind::Text, true, true)
        .map_err(|_| Error::InvalidValue)?;
    let target = ArgumentSpec::new("TARGET", ArgumentKind::Text, true, true)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("LINK", &[source, target]).map_err(|_| Error::InvalidValue)?,
        route(LINK_ROUTE),
    )?;
    let links_path = ArgumentSpec::new("PATH", ArgumentKind::Text, false, true)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("SHOW-LINKS", &[links_path]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_LINKS_ROUTE),
    )?;
    Ok(())
}

fn route(raw: u16) -> RouteId {
    RouteId::new(raw).expect("filesystem route is non-zero")
}

struct TypeBuffer {
    bytes: [u8; MAX_TYPE_OUTPUT_BYTES],
    len: usize,
    source_bytes: usize,
    binary: bool,
    truncated: bool,
}

impl TypeBuffer {
    const fn new(binary: bool) -> Self {
        Self {
            bytes: [0; MAX_TYPE_OUTPUT_BYTES],
            len: 0,
            source_bytes: 0,
            binary,
            truncated: false,
        }
    }
}

impl FileOutput for TypeBuffer {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Status> {
        self.source_bytes = self.source_bytes.saturating_add(bytes.len());
        if self.binary {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            for byte in bytes {
                if self.len + 2 > MAX_TYPE_OUTPUT_BYTES {
                    self.truncated = true;
                    continue;
                }
                self.bytes[self.len] = HEX[(byte >> 4) as usize];
                self.bytes[self.len + 1] = HEX[(byte & 0x0f) as usize];
                self.len += 2;
            }
            return Ok(());
        }
        let available = MAX_TYPE_OUTPUT_BYTES.saturating_sub(self.len);
        let copied = available.min(bytes.len());
        self.bytes[self.len..self.len + copied].copy_from_slice(&bytes[..copied]);
        self.len += copied;
        self.truncated |= copied != bytes.len();
        Ok(())
    }
}

fn write_u32<const CAPACITY: usize>(output: &mut Text<CAPACITY>, value: u32) -> Result<(), Status> {
    let mut digits = [0; 10];
    let mut value = value;
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break
        }
    }
    for digit in digits[..count].iter().rev() {
        output
            .push_char(*digit as char)
            .map_err(|_| Status::NO_SPACE)?;
    }
    Ok(())
}

pub struct FilesystemExecutor<Source, const CAPACITY: usize = 16> {
    source: Source,
    session: ShellSession,
    completions: [Option<Result<StructuredOutput, Status>>; CAPACITY],
}

impl<Source, const CAPACITY: usize> FilesystemExecutor<Source, CAPACITY> {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            session: ShellSession::new(),
            completions: [const { None }; CAPACITY],
        }
    }

    pub const fn session(&self) -> &ShellSession {
        &self.session
    }

    pub const fn session_mut(&mut self) -> &mut ShellSession {
        &mut self.session
    }

    pub const fn source(&self) -> &Source {
        &self.source
    }

    pub const fn source_mut(&mut self) -> &mut Source {
        &mut self.source
    }
}

impl<Source: FilesystemSource, const CAPACITY: usize> CommandExecutor
    for FilesystemExecutor<Source, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .completions
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        self.completions[slot] = Some(self.execute(command));
        ExecutionToken::new((slot + 1) as u64).ok_or(Error::InvalidHandle)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        let slot = token.raw().checked_sub(1)? as usize;
        self.completions.get_mut(slot)?.take()
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let slot = token.raw().checked_sub(1).ok_or(Error::InvalidHandle)? as usize;
        let completion = self.completions.get_mut(slot).ok_or(Error::InvalidHandle)?;
        *completion = None;
        Ok(())
    }
}

impl<Source: FilesystemSource, const CAPACITY: usize> FilesystemExecutor<Source, CAPACITY> {
    pub fn execute_command(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        self.execute(command)
    }

    pub fn list_directory_page(
        &mut self,
        command: CommandCall,
        continuation: Option<u32>,
        output: &mut DirectoryPage,
    ) -> Result<Path, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str().contains(';') {
            return Err(Status::INVALID_ARGUMENT)
        }
        let continuation = continuation.or(integer(command.get("CONTINUATION"))?);
        self.source.list(path.as_str(), continuation, output)?;
        Ok(path)
    }

    pub fn type_file_stream(
        &mut self,
        command: CommandCall,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let binary = boolean(command.get("BINARY"))?;
        self.source.type_file(path.as_str(), binary, output)
    }

    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            DIRECTORY_ROUTE => self.directory(command),
            MKDIR_ROUTE => self.mkdir(command),
            CREATE_FILE_ROUTE => self.create_file(command),
            TYPE_ROUTE => self.type_file(command),
            SET_DEFAULT_ROUTE => self.set_default(command),
            SHOW_DEFAULT_ROUTE => self.default_output(),
            LINK_ROUTE => self.link(command),
            SHOW_LINKS_ROUTE => self.show_links(command),
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn directory(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str().contains(';') {
            return Err(Status::INVALID_ARGUMENT)
        }
        if boolean(command.get("CREATE"))? {
            let metadata = self
                .source
                .create_directory(path.as_str(), boolean(command.get("RECURSIVE"))?)?;
            return metadata_output("created", metadata);
        }
        let mut page = DirectoryPage::new();
        let continuation = integer(command.get("CONTINUATION"))?;
        self.source.list(path.as_str(), continuation, &mut page)?;
        directory_output(path, page, continuation)
    }

    fn mkdir(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str().contains(';') {
            return Err(Status::INVALID_ARGUMENT)
        }
        boolean(command.get("RECURSIVE"))?;
        let metadata = self.source.create_directory(path.as_str(), true)?;
        metadata_output("created", metadata)
    }

    fn create_file(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str() == "/" || path.as_str().contains(';') {
            return Err(Status::INVALID_ARGUMENT)
        }
        metadata_output("created", self.source.create_file(path.as_str())?)
    }

    fn type_file(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let binary = boolean(command.get("BINARY"))?;
        let mut contents = TypeBuffer::new(binary);
        let metadata = self
            .source
            .type_file(path.as_str(), binary, &mut contents)?;
        let mut output = metadata_output("file", metadata)?;
        let visible = core::str::from_utf8(&contents.bytes[..contents.len])
            .map_err(|_| Status::INVALID_ARGUMENT)?;
        let visible = truncate_utf8(visible, 255);
        insert_text(&mut output, "content", visible)?;
        insert_text(
            &mut output,
            "encoding",
            if binary { "hex" } else { "text" },
        )?;
        insert(
            &mut output,
            "content-bytes",
            OutputValue::Unsigned(contents.source_bytes as u64),
        )?;
        insert(
            &mut output,
            "truncated",
            OutputValue::Boolean(contents.truncated || visible.len() != contents.len),
        )?;
        Ok(output)
    }

    fn link(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let source = text(command.get("SOURCE")).ok_or(Status::INVALID_ARGUMENT)?;
        let target = text(command.get("TARGET")).ok_or(Status::INVALID_ARGUMENT)?;
        let source = self.session.resolve(Some(source.as_str()))?;
        let target = self.session.resolve(Some(target.as_str()))?;
        if split_version_selector(target.as_str())?.1.is_some() || target.as_str() == "/" {
            return Err(Status::INVALID_ARGUMENT)
        }
        metadata_output("linked", self.source.link(source.as_str(), target.as_str())?)
    }

    fn show_links(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path = self
            .session
            .resolve(text(command.get("PATH")).as_ref().map(Text::as_str))?;
        let mut links = LinkPage::new();
        let metadata = self.source.list_links(path.as_str(), &mut links)?;
        let mut output = metadata_output("links", metadata)?;
        for (index, link) in links.entries().enumerate() {
            let mut field = Text::<64>::empty();
            field.push_str("link-").map_err(|_| Status::NO_SPACE)?;
            write_u32(&mut field, index as u32)?;
            field.push_str("-path").map_err(|_| Status::NO_SPACE)?;
            insert_text(&mut output, field.as_str(), link.as_str())?;
        }
        Ok(output)
    }

    fn set_default(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path = text(command.get("PATH")).ok_or(Status::INVALID_ARGUMENT)?;
        let directory = self.session.set_default(&mut self.source, path.as_str())?;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "default-directory", directory.as_str())?;
        Ok(output)
    }

    fn default_output(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(
            &mut output,
            "default-directory",
            self.session.default_directory().as_str(),
        )?;
        Ok(output)
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    let end = value.len().min(max_bytes);
    if end == value.len() {
        return value
    }
    let mut boundary = end;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn text(value: Option<Value>) -> Option<Text<{ crate::MAX_TOKEN_BYTES }>> {
    match value {
        Some(Value::Text(value)) => Some(value),
        _ => None,
    }
}

fn boolean(value: Option<Value>) -> Result<bool, Status> {
    match value {
        None => Ok(false),
        Some(Value::Boolean(value)) => Ok(value),
        _ => Err(Status::INVALID_ARGUMENT),
    }
}

fn integer(value: Option<Value>) -> Result<Option<u32>, Status> {
    match value {
        None => Ok(None),
        Some(Value::Integer(value)) => u32::try_from(value)
            .map(Some)
            .map_err(|_| Status::INVALID_ARGUMENT),
        _ => Err(Status::INVALID_ARGUMENT),
    }
}

fn metadata_output(label: &str, metadata: FileMetadata) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", label)?;
    insert_text(&mut output, "path", metadata.path.as_str())?;
    insert_text(&mut output, "type", metadata.file_type.as_str())?;
    insert(&mut output, "size", OutputValue::Unsigned(metadata.size))?;
    insert(
        &mut output,
        "version",
        OutputValue::Unsigned(metadata.version as u64),
    )?;
    insert(
        &mut output,
        "link-count",
        OutputValue::Unsigned(metadata.link_count as u64),
    )?;
    Ok(output)
}

fn directory_output(
    path: Path,
    page: DirectoryPage,
    continuation: Option<u32>,
) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "path", path.as_str())?;
    insert(
        &mut output,
        "entry-count",
        OutputValue::Unsigned(page.len() as u64),
    )?;
    let has_more = page.next.is_some() || page.len() > MAX_VISIBLE_DIRECTORY_ENTRIES;
    let visible_entries = if has_more {
        MAX_VISIBLE_DIRECTORY_ENTRIES - 1
    } else {
        MAX_VISIBLE_DIRECTORY_ENTRIES
    };
    if has_more {
        let next = if page.len() > visible_entries {
            continuation
                .unwrap_or(0)
                .checked_add(visible_entries as u32)
                .ok_or(Status::NO_SPACE)?
        } else {
            page.next.ok_or(Status::NO_SPACE)?
        };
        insert(&mut output, "next", OutputValue::Unsigned(next as u64))?;
    }
    for (index, entry) in page.entries().take(visible_entries).enumerate() {
        let prefix = match index {
            0 => "entry-0-",
            1 => "entry-1-",
            2 => "entry-2-",
            3 => "entry-3-",
            4 => "entry-4-",
            _ => "entry-5-",
        };
        let mut name = Text::<32>::empty();
        name.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
        let mut field = Text::<64>::empty();
        field
            .push_str(name.as_str())
            .map_err(|_| Status::NO_SPACE)?;
        field.push_str("name").map_err(|_| Status::NO_SPACE)?;
        insert_text(&mut output, field.as_str(), entry.name.as_str())?;
        let mut field = Text::<64>::empty();
        field.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
        field.push_str("type").map_err(|_| Status::NO_SPACE)?;
        insert_text(&mut output, field.as_str(), entry.file_type.as_str())?;
        let mut field = Text::<64>::empty();
        field.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
        field.push_str("size").map_err(|_| Status::NO_SPACE)?;
        insert(
            &mut output,
            field.as_str(),
            OutputValue::Unsigned(entry.size),
        )?;
        let mut field = Text::<64>::empty();
        field.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
        field.push_str("version").map_err(|_| Status::NO_SPACE)?;
        insert(
            &mut output,
            field.as_str(),
            OutputValue::Unsigned(entry.version as u64),
        )?;
        let mut field = Text::<64>::empty();
        field.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
        field.push_str("link-count").map_err(|_| Status::NO_SPACE)?;
        insert(
            &mut output,
            field.as_str(),
            OutputValue::Unsigned(entry.link_count as u64),
        )?;
    }
    Ok(output)
}

fn insert_text(output: &mut StructuredOutput, name: &str, value: &str) -> Result<(), Status> {
    insert(
        output,
        name,
        OutputValue::Text(OutputText::new(value).map_err(|_| Status::NO_SPACE)?),
    )
}

fn insert(output: &mut StructuredOutput, name: &str, value: OutputValue) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}

impl core::fmt::Debug for ShellSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ShellSession")
            .field("default_directory", &self.default_directory.as_str())
            .finish()
    }
}

impl core::fmt::Write for Path {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        let end = self.len as usize + value.len();
        if end > MAX_PATH_BYTES {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len as usize..end].copy_from_slice(value.as_bytes());
        self.len = end as u16;
        Ok(())
    }
}
