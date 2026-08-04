use synos_status::Status;
use synos_path_pattern::{Pattern, PatternError, unescape};
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputText, OutputValue, StructuredOutput,
};

use crate::{
    Error, Text,
    file_editor::MAX_EDITOR_BYTES,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_DIRECTORY_PAGE_ENTRIES: usize = 32;
pub const MAX_LINK_PAGE_ENTRIES: usize = 16;
pub const MAX_PATH_COMPLETION_MATCHES: usize = MAX_DIRECTORY_PAGE_ENTRIES;
pub const MAX_WILDCARD_MATCHES: usize = 256;
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
pub const DELETE_ROUTE: u16 = 40;
pub const EDIT_ROUTE: u16 = 41;
pub const RMDIR_ROUTE: u16 = 42;

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
            || (bytes.len() > 1 && bytes.ends_with(b"/"))
            || bytes.windows(2).any(|pair| pair == b"//")
        {
            return Err(Status::INVALID_PATH);
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
    pub is_link: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeleteMetadata {
    pub file: FileMetadata,
    pub shared_data_reachable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryRemovalMetadata {
    pub directory: FileMetadata,
    pub removal_generation: u64,
    pub storage_reclamation_pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: Path,
    pub file_type: EntryType,
    pub size: u64,
    pub version: u32,
    pub link_count: u32,
    pub is_link: bool,
}

pub struct DirectoryPage {
    entries: [Option<DirectoryEntry>; MAX_DIRECTORY_PAGE_ENTRIES],
    count: usize,
    pub next: Option<u32>,
    pub wildcard_matches: usize,
    pub wildcard_failure: Option<Status>,
    pub wildcard_cancelled: bool,
}

/// Result of the shared wildcard expansion contract.
///
/// Future path commands must consume this result before doing filesystem
/// work. Matches are bounded, sorted, and duplicate-free in the supplied
/// `PathCompletionPage`. A failure after one or more matches is partial; a
/// caller must preserve completed mutations and report the returned status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WildcardExpansion {
    pub matches: usize,
    pub failure: Option<Status>,
    pub cancelled: bool,
    pub next: Option<u32>,
}

impl WildcardExpansion {
    pub const fn complete(matches: usize) -> Self {
        Self {
            matches,
            failure: None,
            cancelled: false,
            next: None,
        }
    }

    pub const fn partial(matches: usize, failure: Status) -> Self {
        Self {
            matches,
            failure: Some(failure),
            cancelled: failure.raw() == Status::CANCELLED.raw(),
            next: None,
        }
    }

    pub const fn page(matches: usize, next: Option<u32>) -> Self {
        Self {
            matches,
            failure: None,
            cancelled: false,
            next,
        }
    }

    pub const fn status(self) -> Status {
        if self.cancelled {
            Status::CANCELLED
        } else if self.failure.is_some() {
            Status::PARTIAL_MATCH
        } else {
            Status::NORMAL
        }
    }
}

pub struct PathCompletionPage {
    entries: [Option<Path>; MAX_PATH_COMPLETION_MATCHES],
    count: usize,
}

impl PathCompletionPage {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_PATH_COMPLETION_MATCHES],
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.entries = [None; MAX_PATH_COMPLETION_MATCHES];
        self.count = 0
    }

    pub fn push(&mut self, path: Path) -> Result<(), Status> {
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.as_str() == path.as_str())
        {
            return Ok(())
        }
        if self.count == self.entries.len() {
            return Err(Status::NO_SPACE)
        }
        let insert_at = self.entries[..self.count]
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.as_str() > path.as_str()))
            .unwrap_or(self.count);
        for index in (insert_at..self.count).rev() {
            self.entries[index + 1] = self.entries[index];
        }
        self.entries[insert_at] = Some(path);
        self.count += 1;
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = Path> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.count
    }
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
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.as_str() == path.as_str())
        {
            return Ok(())
        }
        if self.count == self.entries.len() {
            return Err(Status::NO_SPACE)
        }
        let insert_at = self.entries[..self.count]
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.as_str() > path.as_str()))
            .unwrap_or(self.count);
        for index in (insert_at..self.count).rev() {
            self.entries[index + 1] = self.entries[index];
        }
        self.entries[insert_at] = Some(path);
        self.count += 1;
        Ok(())
    }

    pub fn push_unique(&mut self, path: Path) -> Result<(), Status> {
        self.push(path)
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
            wildcard_matches: 0,
            wildcard_failure: None,
            wildcard_cancelled: false,
        }
    }

    pub fn clear(&mut self) {
        self.entries = [None; MAX_DIRECTORY_PAGE_ENTRIES];
        self.count = 0;
        self.next = None;
        self.wildcard_matches = 0;
        self.wildcard_failure = None;
        self.wildcard_cancelled = false
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
    fn complete(
        &mut self,
        _directory: &str,
        _prefix: &str,
        _output: &mut PathCompletionPage,
    ) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }
    fn expand(
        &mut self,
        pattern: &str,
        output: &mut PathCompletionPage,
    ) -> Result<(), Status> {
        match expand_pattern_page(self, pattern, 0, output)? {
            Some(_) => Err(Status::NO_SPACE),
            None => Ok(()),
        }
    }

    /// Fill one stable wildcard page. `pattern` includes its optional version
    /// selector, and `continuation` is the zero-based match ordinal.
    fn expand_page(
        &mut self,
        pattern: &str,
        continuation: u32,
        output: &mut PathCompletionPage,
    ) -> Result<Option<u32>, Status> {
        expand_pattern_page(self, pattern, continuation, output)
    }

    /// Wildcard scans are cooperative. A remote or kernel-backed source can
    /// report cancellation between bounded directory pages.
    fn is_cancelled(&mut self) -> bool {
        false
    }

    fn request_cancel(&mut self) {}
    fn version_exists(&mut self, _path: &str, _version: u32) -> Result<bool, Status> {
        Ok(true)
    }
    fn list(
        &mut self,
        path: &str,
        continuation: Option<u32>,
        output: &mut DirectoryPage,
    ) -> Result<(), Status>;
    fn create_directory(&mut self, path: &str, recursive: bool) -> Result<FileMetadata, Status>;
    fn create_file(&mut self, path: &str) -> Result<FileMetadata, Status>;
    fn remove_directory(&mut self, _path: &str) -> Result<DirectoryRemovalMetadata, Status> {
        Err(Status::NOT_FOUND)
    }
    fn delete(&mut self, _path: &str) -> Result<DeleteMetadata, Status> {
        Err(Status::NOT_FOUND)
    }
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

    fn save_file(&mut self, _path: &str, _contents: &[u8]) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Publish contents as a new version only when the opened source version
    /// is still current. Implementations must make the publish atomic and
    /// return metadata for the committed version.
    fn save_file_if_version(
        &mut self,
        path: &str,
        _expected_version: u32,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        self.save_file(path, contents)
    }

    /// Publish after the user explicitly accepts a stale-source conflict.
    fn save_file_force(
        &mut self,
        path: &str,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        self.save_file(path, contents)
    }
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
            return Err(Status::INVALID_PATH);
        }
        if path_without_version != "/"
            && (path_without_version.ends_with('/')
                || path_without_version
                    .as_bytes()
                    .windows(2)
                    .any(|pair| pair == b"//"))
        {
            return Err(Status::INVALID_PATH);
        }
        if path_without_version.contains('\0') {
            return Err(Status::INVALID_PATH);
        }
        Pattern::parse(path_without_version).map_err(pattern_status)?;
        let mut source = Text::<{ MAX_PATH_BYTES * 2 + 1 }>::empty();
        if !path_without_version.starts_with('/') {
            source
                .push_str(self.default_directory.as_str())
                .map_err(|_| Status::INVALID_PATH)?;
            if !source.as_str().ends_with('/') {
                source
                    .push_char('/')
                    .map_err(|_| Status::INVALID_PATH)?;
            }
        }
        source
            .push_str(path_without_version)
            .map_err(|_| Status::INVALID_PATH)?;
        let resolved = canonicalize(source.as_str())?;
        let mut result = Text::<MAX_PATH_BYTES>::new(resolved.as_str())
            .map_err(|_| Status::INVALID_ARGUMENT)?;
        if let Some(version) = version {
            result
                .push_char(';')
                .map_err(|_| Status::INVALID_PATH)?;
            let mut version_text = Text::<10>::empty();
            write_u32(&mut version_text, version)?;
            result
                .push_str(version_text.as_str())
                .map_err(|_| Status::INVALID_PATH)?;
        }
        Path::new(result.as_str())
    }

    pub fn set_default<S: FilesystemSource>(
        &mut self,
        source: &mut S,
        path: &str,
    ) -> Result<Path, Status> {
        if path.contains(';') || contains_wildcard(path) {
            return Err(Status::INVALID_ARGUMENT);
        }
        let resolved = self.resolve(Some(path))?;
        let resolved = literal_path(resolved.as_str())?;
        let mut page = DirectoryPage::new();
        source.list(resolved.as_str(), None, &mut page)?;
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
                return Err(Status::INVALID_PATH);
            }
            result.len = component_starts[component_count - 1] as u16;
            component_count -= 1;
            cursor = result.len as usize;
            continue;
        }
        if component.contains('\0') || component_count == component_starts.len() {
            return Err(Status::INVALID_PATH);
        }
        let start = if cursor == 1 { 1 } else { cursor + 1 };
        let end = start
            .checked_add(component.len())
            .ok_or(Status::INVALID_PATH)?;
        if end > MAX_PATH_BYTES {
            return Err(Status::INVALID_PATH);
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

const MAX_WILDCARD_SCAN: usize = 4096;

fn pattern_status(error: PatternError) -> Status {
    match error {
        PatternError::TooLong => Status::NO_SPACE,
        PatternError::InvalidPath => Status::INVALID_PATH,
        PatternError::Empty
        | PatternError::TrailingEscape
        | PatternError::UnterminatedClass
        | PatternError::EmptyClass
        | PatternError::InvalidRange => Status::INVALID_PATTERN,
    }
}

fn literal_path(value: &str) -> Result<Path, Status> {
    let mut bytes = [0; MAX_PATH_BYTES];
    let length = unescape(value, &mut bytes).map_err(pattern_status)?;
    let value = core::str::from_utf8(&bytes[..length]).map_err(|_| Status::INVALID_PATH)?;
    Path::new(value)
}

fn child_path(directory: &str, name: &str) -> Result<Path, Status> {
    let mut path = Text::<{ MAX_PATH_BYTES }>::empty();
    if directory == "/" {
        path.push_char('/').map_err(|_| Status::NO_SPACE)?;
    } else {
        path.push_str(directory).map_err(|_| Status::NO_SPACE)?;
        path.push_char('/').map_err(|_| Status::NO_SPACE)?;
    }
    path.push_str(name).map_err(|_| Status::NO_SPACE)?;
    Path::new(path.as_str())
}

fn selected_path(path: Path, version: Option<u32>) -> Result<Path, Status> {
    let Some(version) = version.filter(|version| *version != 0) else {
        return Ok(path)
    };
    let mut value = Text::<{ MAX_PATH_BYTES }>::empty();
    value.push_str(path.as_str()).map_err(|_| Status::NO_SPACE)?;
    value.push_char(';').map_err(|_| Status::NO_SPACE)?;
    write_u32(&mut value, version)?;
    Path::new(value.as_str())
}

fn insert_wildcard_match(
    matches: &mut [Option<Path>; MAX_WILDCARD_MATCHES],
    count: &mut usize,
    path: Path,
) -> Result<(), Status> {
    if matches[..*count]
        .iter()
        .flatten()
        .any(|entry| entry.as_str() == path.as_str())
    {
        return Ok(())
    }
    if *count == matches.len() {
        return Err(Status::NO_SPACE)
    }
    let insert_at = matches[..*count]
        .iter()
        .position(|entry| entry.is_some_and(|entry| entry.as_str() > path.as_str()))
        .unwrap_or(*count);
    for index in (insert_at..*count).rev() {
        matches[index + 1] = matches[index];
    }
    matches[insert_at] = Some(path);
    *count += 1;
    Ok(())
}

fn expand_pattern_page<S: FilesystemSource + ?Sized>(
    source: &mut S,
    pattern: &str,
    continuation: u32,
    output: &mut PathCompletionPage,
) -> Result<Option<u32>, Status> {
    let (base, version) = split_version_selector(pattern)?;
    let pattern = Pattern::parse(base).map_err(pattern_status)?;
    output.clear();
    if !pattern.has_magic() {
        if continuation == 0 {
            output.push(selected_path(literal_path(pattern.as_str())?, version)?)?;
        }
        return Ok(None)
    }

    let mut matches = [None; MAX_WILDCARD_MATCHES];
    let mut match_count = 0;
    let mut scanned = 0;
    let root = wildcard_root(pattern.as_str())?;
    let recurse = wildcard_has_later_component(pattern.as_str());
    let scan = expand_directory(
        source,
        pattern,
        version,
        root.as_str(),
        &mut matches,
        &mut match_count,
        recurse,
        0,
        &mut scanned,
    );

    let start = continuation as usize;
    if let Err(status) = scan {
        if start < match_count {
            let end = start
                .saturating_add(MAX_PATH_COMPLETION_MATCHES)
                .min(match_count);
            for path in matches[start..end].iter().flatten().copied() {
                output.push(path)?;
            }
        }
        return Err(status)
    }
    if start >= match_count {
        return Ok(None)
    }
    let end = start
        .saturating_add(MAX_PATH_COMPLETION_MATCHES)
        .min(match_count);
    for path in matches[start..end].iter().flatten().copied() {
        output.push(path)?;
    }
    Ok((end < match_count).then_some(end as u32))
}

fn wildcard_has_later_component(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'*' | b'?' | b'[' => return bytes[index..].contains(&b'/'),
            _ => index += 1,
        }
    }
    false
}

fn wildcard_root(pattern: &str) -> Result<Path, Status> {
    let bytes = pattern.as_bytes();
    let mut index = 0;
    let mut wildcard_start = None;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'*' | b'?' | b'[' => {
                wildcard_start = Some(index);
                break
            }
            _ => index += 1,
        }
    }
    let wildcard_start = wildcard_start.ok_or(Status::INVALID_ARGUMENT)?;
    let prefix = pattern[..wildcard_start].rsplit_once('/').map_or("/", |(prefix, _)| {
        if prefix.is_empty() { "/" } else { prefix }
    });
    literal_path(prefix)
}

fn expand_directory<S: FilesystemSource + ?Sized>(
    source: &mut S,
    pattern: Pattern<'_>,
    version: Option<u32>,
    directory: &str,
    matches: &mut [Option<Path>; MAX_WILDCARD_MATCHES],
    match_count: &mut usize,
    recurse: bool,
    depth: usize,
    scanned: &mut usize,
) -> Result<(), Status> {
    if depth >= MAX_PATH_BYTES / 2 {
        return Err(Status::NO_SPACE)
    }
    let mut continuation = None;
    loop {
        if source.is_cancelled() {
            return Err(Status::CANCELLED)
        }
        let mut page = DirectoryPage::new();
        source.list(directory, continuation, &mut page)?;
        for entry in page.entries() {
            if source.is_cancelled() {
                return Err(Status::CANCELLED)
            }
            *scanned = scanned.saturating_add(1);
            if *scanned > MAX_WILDCARD_SCAN {
                return Err(Status::NO_SPACE)
            }
            let child = child_path(directory, entry.name.as_str())?;
            if pattern.matches(child.as_str()) {
                let selected = version
                    .filter(|version| *version != 0)
                    .map_or(Ok(true), |version| {
                        source.version_exists(child.as_str(), version)
                    })?;
                if selected {
                    let selected_path = selected_path(child, version)?;
                    insert_wildcard_match(matches, match_count, selected_path)?;
                }
            }
            if recurse && entry.file_type == EntryType::Directory {
                expand_directory(
                    source,
                    pattern,
                    version,
                    child.as_str(),
                    matches,
                    match_count,
                    recurse,
                    depth + 1,
                    scanned,
                )?;
            }
        }
        let Some(next) = page.next else { break };
        continuation = Some(next);
    }
    Ok(())
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
        CommandSpec::new("RMDIR", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(RMDIR_ROUTE),
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
    registry.register(
        CommandSpec::new("DELETE", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(DELETE_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("EDIT", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(EDIT_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("EDT", &[required_path]).map_err(|_| Error::InvalidValue)?,
        route(EDIT_ROUTE),
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

impl TypeBuffer {
    fn separator(&mut self, path: &str) -> Result<(), Status> {
        const SEPARATOR: &[u8] = b"\n\n--- ";
        self.write(SEPARATOR)?;
        self.write(path.as_bytes())?;
        self.write(b" ---\n")
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
        self.source.request_cancel();
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
        let (base_path, version) = split_version_selector(path.as_str())?;
        if contains_wildcard(base_path) {
            let mut matches = PathCompletionPage::new();
            let continuation = continuation.or(integer(command.get("CONTINUATION"))?);
            let expansion =
                expand_paths_page(&mut self.source, path.as_str(), continuation, &mut matches)?;
            if matches.len() == 0 {
                return Err(Status::NOT_FOUND)
            }
            output.clear();
            output.wildcard_matches = expansion.matches;
            output.wildcard_failure = expansion.failure;
            output.wildcard_cancelled = expansion.cancelled;
            let mut next_match = continuation.unwrap_or(0);
            for matched in matches.entries() {
                if self.source.is_cancelled() {
                    output.wildcard_failure = Some(Status::CANCELLED);
                    output.wildcard_cancelled = true;
                    break
                }
                let (matched_path, selected_version) = split_version_selector(matched.as_str())?;
                let Some((parent, name)) = matched_path.rsplit_once('/') else {
                    return Err(Status::NOT_FOUND)
                };
                let parent = if parent.is_empty() { "/" } else { parent };
                let mut source_continuation = None;
                loop {
                    let mut page = DirectoryPage::new();
                    match self.source.list(parent, source_continuation, &mut page) {
                        Ok(()) => {}
                        Err(status) if output.len() != 0 => {
                            output.wildcard_failure = Some(status);
                            break
                        }
                        Err(status) => return Err(status),
                    }
                    for entry in page.entries() {
                        if entry.name.as_str() != name
                            || !selected_version
                                .map_or(true, |version| version == 0 || version == entry.version)
                        {
                            continue
                        }
                        if output.len() == MAX_DIRECTORY_PAGE_ENTRIES {
                            output.next = Some(next_match.saturating_add(1));
                            return Ok(path)
                        }
                        output.push(entry)?;
                    }
                    let Some(next) = page.next else { break };
                    source_continuation = Some(next);
                }
                if output.wildcard_failure.is_some() {
                    break
                }
                next_match += 1;
            }
            output.next = expansion.next;
            if output.len() == 0 {
                return Err(if output.wildcard_cancelled {
                    Status::CANCELLED
                } else if output.wildcard_failure.is_some() {
                    Status::PARTIAL_MATCH
                } else {
                    Status::NOT_FOUND
                })
            }
            return Ok(path)
        }
        if version.is_none() {
            let continuation = continuation.or(integer(command.get("CONTINUATION"))?);
            match self.source.list(base_path, continuation, output) {
                Ok(()) => return Ok(path),
                Err(Status::NOT_FOUND) => {}
                Err(status) => return Err(status),
            }
        }

        let Some((parent, name)) = base_path.rsplit_once('/') else {
            return Err(Status::NOT_FOUND)
        };
        let parent = if parent.is_empty() { "/" } else { parent };
        let continuation = continuation.or(integer(command.get("CONTINUATION"))?);
        let mut source_continuation = continuation;
        output.clear();
        loop {
            let mut page = DirectoryPage::new();
            self.source
                .list(parent, source_continuation, &mut page)?;
            for entry in page.entries() {
                if entry.name.as_str() != name
                    || !version.map_or(true, |version| version == 0 || version == entry.version)
                {
                    continue
                }
                output.push(entry)?;
            }
            let Some(next) = page.next else { break };
            source_continuation = Some(next);
        }
        if output.len() == 0 {
            return Err(Status::NOT_FOUND)
        }
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

    pub fn edit_file_load(
        &mut self,
        command: CommandCall,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        self.open_edit_file(path.as_str(), output)
    }

    pub fn edit_file_save(
        &mut self,
        command: CommandCall,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let (base_path, _) = split_version_selector(path.as_str())?;
        let metadata = self.source.save_file(base_path, contents)?;
        validate_saved_metadata(base_path, 0, contents, metadata)
    }

    pub fn edit_file_save_if_version(
        &mut self,
        command: CommandCall,
        expected_version: u32,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let (base_path, selected_version) = split_version_selector(path.as_str())?;
        let save_path = if selected_version.is_some_and(|version| {
            version != 0 && version == expected_version
        }) {
            path.as_str()
        } else {
            base_path
        };
        let metadata = self
            .source
            .save_file_if_version(save_path, expected_version, contents)?;
        validate_saved_metadata(base_path, expected_version, contents, metadata)
    }

    pub fn edit_file_force_save(
        &mut self,
        command: CommandCall,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let (base_path, _) = split_version_selector(path.as_str())?;
        let metadata = self.source.save_file_force(base_path, contents)?;
        validate_saved_metadata(base_path, 0, contents, metadata)
    }

    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            DIRECTORY_ROUTE => self.directory(command),
            MKDIR_ROUTE => self.mkdir(command),
            RMDIR_ROUTE => self.remove_directory(command),
            CREATE_FILE_ROUTE => self.create_file(command),
            TYPE_ROUTE => self.type_file(command),
            SET_DEFAULT_ROUTE => self.set_default(command),
            SHOW_DEFAULT_ROUTE => self.default_output(),
            LINK_ROUTE => self.link(command),
            SHOW_LINKS_ROUTE => self.show_links(command),
            DELETE_ROUTE => self.delete(command),
            EDIT_ROUTE => self.edit(command),
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn edit(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let mut sink = EditOpenSink::new();
        let metadata = self.open_edit_file(path.as_str(), &mut sink)?;
        core::str::from_utf8(&sink.bytes[..sink.len]).map_err(|_| Status::INVALID_ARGUMENT)?;
        if metadata.file_type != EntryType::File || metadata.size != sink.len as u64 {
            return Err(Status::CORRUPT)
        }
        metadata_output("opened", metadata)
    }

    fn open_edit_file(
        &mut self,
        path: &str,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        match self.source.type_file(path, false, output) {
            Ok(metadata) => Ok(metadata),
            Err(Status::NOT_FOUND) => {
                let (base_path, version) = split_version_selector(path)?;
                if version.is_some_and(|version| version != 0) {
                    return Err(Status::NOT_FOUND)
                }
                match self.source.create_file(base_path) {
                    Ok(_) | Err(Status::ALREADY_EXISTS) => {
                        self.source.type_file(base_path, false, output)
                    }
                    Err(status) => Err(status),
                }
            }
            Err(status) => Err(status),
        }
    }

    fn directory(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let create = boolean(command.get("CREATE"))?;
        let recursive = boolean(command.get("RECURSIVE"))?;
        let continuation = integer(command.get("CONTINUATION"))?;
        if path.as_str().contains(';') {
            return Err(Status::INVALID_PATH)
        }
        if contains_wildcard(path.as_str()) {
            if create || recursive {
                return Err(Status::INVALID_ARGUMENT)
            }
            let mut page = DirectoryPage::new();
            let path = self.list_directory_page(command, continuation, &mut page)?;
            return directory_output(path, page, continuation)
        }
        let path = literal_path(path.as_str())?;
        if create {
            let metadata = self
                .source
                .create_directory(path.as_str(), recursive)?;
            return metadata_output("created", metadata);
        }
        let mut page = DirectoryPage::new();
        self.source.list(path.as_str(), continuation, &mut page)?;
        directory_output(path, page, continuation)
    }

    fn mkdir(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str().contains(';') || contains_wildcard(path.as_str()) {
            return Err(Status::INVALID_ARGUMENT)
        }
        boolean(command.get("RECURSIVE"))?;
        let path = literal_path(path.as_str())?;
        let metadata = self.source.create_directory(path.as_str(), true)?;
        metadata_output("created", metadata)
    }

    fn remove_directory(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH")).ok_or(Status::INVALID_ARGUMENT)?;
        let path = self.session.resolve(Some(path_value.as_str()))?;
        if is_path_or_descendant(path.as_str(), self.session.default_directory().as_str()) {
            return Err(Status::ACCESS_DENIED)
        }
        if path.as_str() == "/" {
            return Err(Status::INVALID_PATH)
        }
        if path.as_str().contains(';') || contains_wildcard(path.as_str()) {
            return Err(Status::INVALID_ARGUMENT)
        }
        let path = literal_path(path.as_str())?;
        self.source.remove_directory(path.as_str())?;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "operation", "removed")?;
        insert_text(&mut output, "path", path.as_str())?;
        Ok(output)
    }

    fn create_file(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        if path.as_str() == "/"
            || path.as_str().contains(';')
            || contains_wildcard(path.as_str())
        {
            return Err(Status::INVALID_ARGUMENT)
        }
        let path = literal_path(path.as_str())?;
        metadata_output("created", self.source.create_file(path.as_str())?)
    }

    fn type_file(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH"));
        let path = self
            .session
            .resolve(path_value.as_ref().map(Text::as_str))?;
        let binary = boolean(command.get("BINARY"))?;
        let mut matches = PathCompletionPage::new();
        let expansion = expand_paths(&mut self.source, path.as_str(), &mut matches)?;
        if matches.len() == 0 {
            return Err(Status::NOT_FOUND)
        }
        let mut contents = TypeBuffer::new(binary);
        let mut metadata = None;
        let mut processed = 0usize;
        let mut failure = expansion.failure;
        let mut cancelled = expansion.cancelled;
        for (index, matched) in matches.entries().enumerate() {
            if self.source.is_cancelled() {
                failure = Some(Status::CANCELLED);
                cancelled = true;
                break
            }
            if index != 0 {
                if let Err(status) = contents.separator(matched.as_str()) {
                    failure = Some(status);
                    break
                }
            }
            match self
                .source
                .type_file(matched.as_str(), binary, &mut contents)
            {
                Ok(current) => {
                    metadata = Some(current);
                    processed += 1;
                }
                Err(status) => {
                    failure = Some(status);
                    break
                }
            }
        }
        let metadata = metadata.ok_or(failure.unwrap_or(Status::NOT_FOUND))?;
        let status = if cancelled {
            Status::CANCELLED
        } else if failure.is_some() {
            Status::PARTIAL_MATCH
        } else {
            Status::NORMAL
        };
        let mut output = metadata_output_with_status("file", metadata, status)?;
        insert(
            &mut output,
            "match-count",
            OutputValue::Unsigned(matches.len() as u64),
        )?;
        insert(
            &mut output,
            "processed-count",
            OutputValue::Unsigned(processed as u64),
        )?;
        insert(
            &mut output,
            "failed-count",
            OutputValue::Unsigned(u64::from(failure.is_some())),
        )?;
        insert(
            &mut output,
            "partial",
            OutputValue::Boolean(failure.is_some()),
        )?;
        insert(
            &mut output,
            "cancelled",
            OutputValue::Boolean(cancelled),
        )?;
        if let Some(status) = failure {
            insert(
                &mut output,
                "failure-status",
                OutputValue::Unsigned(status.raw() as u64),
            )?;
        }
        for (index, matched) in matches.entries().enumerate().take(8) {
            let mut field = Text::<64>::empty();
            field.push_str("match-").map_err(|_| Status::NO_SPACE)?;
            write_u32(&mut field, index as u32)?;
            field.push_str("-path").map_err(|_| Status::NO_SPACE)?;
            insert_text(&mut output, field.as_str(), matched.as_str())?;
        }
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

    fn delete(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path_value = text(command.get("PATH")).ok_or(Status::INVALID_ARGUMENT)?;
        let path = self.session.resolve(Some(path_value.as_str()))?;
        if path.as_str() == "/" {
            return Err(Status::INVALID_PATH);
        }
        let mut matches = PathCompletionPage::new();
        let expansion = expand_paths(&mut self.source, path.as_str(), &mut matches)?;
        if matches.len() == 0 {
            return Err(Status::NOT_FOUND)
        }
        let mut deleted = None;
        let mut processed = 0usize;
        let mut failure = expansion.failure;
        let mut cancelled = expansion.cancelled;
        for matched in matches.entries() {
            if self.source.is_cancelled() {
                failure = Some(Status::CANCELLED);
                cancelled = true;
                break
            }
            match self.source.delete(matched.as_str()) {
                Ok(current) => {
                    deleted = Some(current);
                    processed += 1;
                }
                Err(status) => {
                    failure = Some(status);
                    break
                }
            }
        }
        let deleted = deleted.ok_or(failure.unwrap_or(Status::NOT_FOUND))?;
        let status = if cancelled {
            Status::CANCELLED
        } else if failure.is_some() {
            Status::PARTIAL_MATCH
        } else {
            Status::NORMAL
        };
        let mut output = metadata_output_with_status("deleted", deleted.file, status)?;
        insert(
            &mut output,
            "match-count",
            OutputValue::Unsigned(matches.len() as u64),
        )?;
        insert(
            &mut output,
            "processed-count",
            OutputValue::Unsigned(processed as u64),
        )?;
        insert(
            &mut output,
            "failed-count",
            OutputValue::Unsigned(u64::from(failure.is_some())),
        )?;
        insert(
            &mut output,
            "partial",
            OutputValue::Boolean(failure.is_some()),
        )?;
        insert(
            &mut output,
            "cancelled",
            OutputValue::Boolean(cancelled),
        )?;
        if let Some(status) = failure {
            insert(
                &mut output,
                "failure-status",
                OutputValue::Unsigned(status.raw() as u64),
            )?;
        }
        insert(
            &mut output,
            "shared-data-reachable",
            OutputValue::Boolean(deleted.shared_data_reachable),
        )?;
        Ok(output)
    }

    fn link(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let source = text(command.get("SOURCE")).ok_or(Status::INVALID_ARGUMENT)?;
        let target = text(command.get("TARGET")).ok_or(Status::INVALID_ARGUMENT)?;
        let source = self.session.resolve(Some(source.as_str()))?;
        let target = self.session.resolve(Some(target.as_str()))?;
        if contains_wildcard(source.as_str()) || contains_wildcard(target.as_str()) {
            return Err(Status::INVALID_ARGUMENT)
        }
        let source = literal_path(source.as_str())?;
        let target = literal_path(target.as_str())?;
        if split_version_selector(target.as_str())?.1.is_some() || target.as_str() == "/" {
            return Err(Status::INVALID_PATH)
        }
        let metadata = self.source.link(source.as_str(), target.as_str())?;
        let mut output = metadata_output("linked", metadata)?;
        insert_text(&mut output, "source", source.as_str())?;
        Ok(output)
    }

    fn show_links(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let path = self
            .session
            .resolve(text(command.get("PATH")).as_ref().map(Text::as_str))?;
        let mut matches = PathCompletionPage::new();
        let expansion = expand_paths(&mut self.source, path.as_str(), &mut matches)?;
        if matches.len() == 0 {
            return Err(Status::NOT_FOUND)
        }
        let mut links = LinkPage::new();
        let mut metadata = None;
        let mut processed = 0usize;
        let mut failure = expansion.failure;
        let mut cancelled = expansion.cancelled;
        for matched in matches.entries() {
            if self.source.is_cancelled() {
                failure = Some(Status::CANCELLED);
                cancelled = true;
                break
            }
            let mut current = LinkPage::new();
            match self.source.list_links(matched.as_str(), &mut current) {
                Ok(current_metadata) => {
                    metadata = Some(current_metadata);
                    processed += 1;
                }
                Err(status) => {
                    failure = Some(status);
                    break
                }
            }
            for link in current.entries() {
                if let Err(status) = links.push_unique(link) {
                    failure = Some(status);
                    break
                }
            }
            if failure.is_some() {
                break
            }
        }
        let metadata = metadata.ok_or(failure.unwrap_or(Status::NOT_FOUND))?;
        let status = if cancelled {
            Status::CANCELLED
        } else if failure.is_some() {
            Status::PARTIAL_MATCH
        } else {
            Status::NORMAL
        };
        let mut output = metadata_output_with_status("links", metadata, status)?;
        insert(
            &mut output,
            "match-count",
            OutputValue::Unsigned(matches.len() as u64),
        )?;
        insert(
            &mut output,
            "processed-count",
            OutputValue::Unsigned(processed as u64),
        )?;
        insert(
            &mut output,
            "failed-count",
            OutputValue::Unsigned(u64::from(failure.is_some())),
        )?;
        insert(
            &mut output,
            "partial",
            OutputValue::Boolean(failure.is_some()),
        )?;
        insert(
            &mut output,
            "cancelled",
            OutputValue::Boolean(cancelled),
        )?;
        if let Some(status) = failure {
            insert(
                &mut output,
                "failure-status",
                OutputValue::Unsigned(status.raw() as u64),
            )?;
        }
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

struct EditOpenSink {
    bytes: [u8; MAX_EDITOR_BYTES],
    len: usize,
}

impl EditOpenSink {
    const fn new() -> Self {
        Self {
            bytes: [0; MAX_EDITOR_BYTES],
            len: 0,
        }
    }
}

impl FileOutput for EditOpenSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Status> {
        let end = self.len.checked_add(bytes.len()).ok_or(Status::NO_SPACE)?;
        if end > self.bytes.len() {
            return Err(Status::NO_SPACE)
        }
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

fn validate_saved_metadata(
    requested_path: &str,
    expected_version: u32,
    contents: &[u8],
    metadata: FileMetadata,
) -> Result<FileMetadata, Status> {
    let (requested_path, _) = split_version_selector(requested_path)?;
    if metadata.path.as_str() != requested_path
        || metadata.file_type != EntryType::File
        || metadata.version == 0
        || metadata.size != contents.len() as u64
        || (expected_version != 0 && metadata.version <= expected_version)
    {
        return Err(Status::CORRUPT)
    }
    Ok(metadata)
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
    metadata_output_with_status(label, metadata, Status::NORMAL)
}

fn metadata_output_with_status(
    label: &str,
    metadata: FileMetadata,
    status: Status,
) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(status);
    insert_text(&mut output, "operation", label)?;
    insert_text(&mut output, "path", metadata.path.as_str())?;
    insert_text(
        &mut output,
        "type",
        if metadata.is_link { "LINK" } else { metadata.file_type.as_str() },
    )?;
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
    let wildcard_status = if page.wildcard_cancelled {
        Status::CANCELLED
    } else if page.wildcard_failure.is_some() {
        Status::PARTIAL_MATCH
    } else {
        Status::NORMAL
    };
    let mut output = StructuredOutput::new(wildcard_status);
    insert_text(&mut output, "path", path.as_str())?;
    if page.wildcard_matches != 0 {
        insert(
            &mut output,
            "match-count",
            OutputValue::Unsigned(page.wildcard_matches as u64),
        )?;
        insert(
            &mut output,
            "processed-count",
            OutputValue::Unsigned(page.len() as u64),
        )?;
        insert(
            &mut output,
            "failed-count",
            OutputValue::Unsigned(u64::from(page.wildcard_failure.is_some())),
        )?;
        insert(
            &mut output,
            "partial",
            OutputValue::Boolean(page.wildcard_failure.is_some()),
        )?;
        insert(
            &mut output,
            "cancelled",
            OutputValue::Boolean(page.wildcard_cancelled),
        )?;
        if let Some(status) = page.wildcard_failure {
            insert(
                &mut output,
                "failure-status",
                OutputValue::Unsigned(status.raw() as u64),
            )?;
        }
    }
    insert(
        &mut output,
        "entry-count",
        OutputValue::Unsigned(page.len() as u64),
    )?;
    let max_visible_entries = if page.wildcard_matches != 0 {
        MAX_VISIBLE_DIRECTORY_ENTRIES - 2
    } else {
        MAX_VISIBLE_DIRECTORY_ENTRIES
    };
    let has_more = page.next.is_some() || page.len() > max_visible_entries;
    let visible_entries = if has_more {
        max_visible_entries - 1
    } else {
        max_visible_entries
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
        insert_text(
            &mut output,
            field.as_str(),
            if entry.is_link { "LINK" } else { entry.file_type.as_str() },
        )?;
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

fn contains_wildcard(path: &str) -> bool {
    Pattern::parse(path).map_or(true, |pattern| pattern.has_magic())
}

/// Expand one resolved path using the shared wildcard contract.
///
/// Literal paths are returned unchanged after unescaping. Wildcard paths are
/// expanded through the source's bounded, capability-filtered implementation.
/// Numeric version selectors are applied after expansion, so an exact selector
/// never falls back to another retained or latest version.
pub fn expand_paths<S: FilesystemSource + ?Sized>(
    source: &mut S,
    path: &str,
    output: &mut PathCompletionPage,
) -> Result<WildcardExpansion, Status> {
    let expansion = expand_paths_page(source, path, None, output)?;
    if let Some(next) = expansion.next {
        return Ok(WildcardExpansion {
            matches: expansion.matches,
            failure: Some(Status::NO_SPACE),
            cancelled: false,
            next: Some(next),
        })
    }
    Ok(expansion)
}

/// Expand one path page without losing the stable match ordinal used for the
/// next request. The page itself is bounded by `MAX_PATH_COMPLETION_MATCHES`.
pub fn expand_paths_page<S: FilesystemSource + ?Sized>(
    source: &mut S,
    path: &str,
    continuation: Option<u32>,
    output: &mut PathCompletionPage,
) -> Result<WildcardExpansion, Status> {
    output.clear();
    match source.expand_page(path, continuation.unwrap_or(0), output) {
        Ok(next) => Ok(WildcardExpansion::page(output.len(), next)),
        Err(status) if output.len() != 0 => {
            Ok(WildcardExpansion::partial(output.len(), status))
        }
        Err(status) => Err(status),
    }
}

fn is_path_or_descendant(path: &str, candidate: &str) -> bool {
    candidate == path
        || path == "/"
        || candidate
            .strip_prefix(path)
            .is_some_and(|suffix| suffix.starts_with('/'))
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink {
        bytes: [u8; 64],
        len: usize,
    }

    impl Sink {
        const fn new() -> Self {
            Self {
                bytes: [0; 64],
                len: 0,
            }
        }
    }

    impl FileOutput for Sink {
        fn write(&mut self, bytes: &[u8]) -> Result<(), Status> {
            let end = self.len.checked_add(bytes.len()).ok_or(Status::NO_SPACE)?;
            if end > self.bytes.len() {
                return Err(Status::NO_SPACE);
            }
            self.bytes[self.len..end].copy_from_slice(bytes);
            self.len = end;
            Ok(())
        }
    }

    struct MockFilesystem {
        bytes: [u8; 64],
        len: usize,
        version: u32,
        present: bool,
        conditional_failure: bool,
        force_failure: bool,
        saves: usize,
    }

    impl MockFilesystem {
        fn new(contents: &[u8], version: u32) -> Self {
            let mut filesystem = Self {
                bytes: [0; 64],
                len: contents.len(),
                version,
                present: true,
                conditional_failure: false,
                force_failure: false,
                saves: 0,
            };
            filesystem.bytes[..contents.len()].copy_from_slice(contents);
            filesystem
        }

        fn metadata(path: &str, file_type: EntryType, size: usize, version: u32) -> FileMetadata {
            FileMetadata {
                path: Path::new(path).expect("valid mock path"),
                file_type,
                size: size as u64,
                version,
                link_count: 1,
                is_link: false,
            }
        }

        fn publish(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
            let (base_path, _) = split_version_selector(path)?;
            if contents.len() > self.bytes.len() {
                return Err(Status::NO_SPACE);
            }
            self.bytes[..contents.len()].copy_from_slice(contents);
            self.len = contents.len();
            self.version = self.version.saturating_add(1).max(1);
            self.present = true;
            self.saves += 1;
            Ok(Self::metadata(base_path, EntryType::File, contents.len(), self.version))
        }
    }

    impl FilesystemSource for MockFilesystem {
        fn directory_exists(&mut self, path: &str) -> Result<bool, Status> {
            Ok(path == "/" || path == "/data")
        }

        fn list(
            &mut self,
            _path: &str,
            _continuation: Option<u32>,
            output: &mut DirectoryPage,
        ) -> Result<(), Status> {
            output.clear();
            Ok(())
        }

        fn create_directory(&mut self, path: &str, _recursive: bool) -> Result<FileMetadata, Status> {
            Ok(Self::metadata(path, EntryType::Directory, 0, 1))
        }

        fn create_file(&mut self, path: &str) -> Result<FileMetadata, Status> {
            if self.present {
                return Err(Status::ALREADY_EXISTS);
            }
            self.present = true;
            self.version = 1;
            Ok(Self::metadata(path, EntryType::File, 0, self.version))
        }

        fn remove_directory(&mut self, path: &str) -> Result<DirectoryRemovalMetadata, Status> {
            if path != "/data" {
                return Err(Status::NOT_FOUND)
            }
            Ok(DirectoryRemovalMetadata {
                directory: Self::metadata(path, EntryType::Directory, 0, 1),
                removal_generation: 2,
                storage_reclamation_pending: true,
            })
        }

        fn delete(&mut self, path: &str) -> Result<DeleteMetadata, Status> {
            let (base_path, selector) = split_version_selector(path)?;
            if base_path == "/data/directory" {
                return Err(Status::NOT_DIRECTORY)
            }
            if !self.present {
                return Err(Status::NOT_FOUND)
            }
            let version = selector.filter(|version| *version != 0).unwrap_or(self.version);
            if version > self.version {
                return Err(Status::NOT_FOUND)
            }
            self.present = false;
            Ok(DeleteMetadata {
                file: Self::metadata(base_path, EntryType::File, self.len, version),
                shared_data_reachable: false,
            })
        }

        fn type_file(
            &mut self,
            path: &str,
            _binary: bool,
            output: &mut dyn FileOutput,
        ) -> Result<FileMetadata, Status> {
            if !self.present {
                return Err(Status::NOT_FOUND);
            }
            output.write(&self.bytes[..self.len])?;
            Ok(Self::metadata(path, EntryType::File, self.len, self.version))
        }

        fn save_file(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
            self.publish(path, contents)
        }

        fn save_file_if_version(
            &mut self,
            path: &str,
            expected_version: u32,
            contents: &[u8],
        ) -> Result<FileMetadata, Status> {
            if self.conditional_failure || expected_version != self.version {
                return Err(Status::CONFLICT);
            }
            self.publish(path, contents)
        }

        fn save_file_force(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
            if self.force_failure {
                return Err(Status::NO_SPACE);
            }
            self.publish(path, contents)
        }
    }

    fn command(line: &str) -> CommandCall {
        let mut registry = CommandRegistry::<16>::new();
        register_filesystem_commands(&mut registry).expect("filesystem commands");
        registry.parse(line).expect("valid filesystem command").stage(0).expect("command stage")
    }

    fn output_value(output: &StructuredOutput, name: &str) -> Option<OutputValue> {
        output
            .fields()
            .find(|field| field.name.as_str() == name)
            .map(|field| field.value)
    }

    #[test]
    fn edit_load_creates_missing_file_and_preserves_versioned_contents() {
        let mut filesystem = MockFilesystem::new(&[], 0);
        filesystem.present = false;
        let mut executor = FilesystemExecutor::<_, 16>::new(filesystem);
        let mut sink = Sink::new();

        let metadata = executor
            .edit_file_load(command("EDIT /data/new-note"), &mut sink)
            .expect("missing file is created for edit");

        assert_eq!(metadata.version, 1);
        assert_eq!(metadata.size, 0);
        assert_eq!(sink.len, 0);
        assert!(executor.source().present);
    }

    #[test]
    fn conditional_save_publishes_a_new_version_and_detects_conflicts() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"old", 1));
        let metadata = executor
            .edit_file_save_if_version(command("EDIT /data/note;1"), 1, b"new")
            .expect("current version saves");

        assert_eq!(metadata.path.as_str(), "/data/note");
        assert_eq!(metadata.version, 2);
        assert_eq!(&executor.source().bytes[..executor.source().len], b"new");

        executor.source_mut().conditional_failure = true;
        let result = executor.edit_file_save_if_version(command("EDIT /data/note"), 2, b"lost");
        assert_eq!(result, Err(Status::CONFLICT));
        assert_eq!(&executor.source().bytes[..executor.source().len], b"new");
        assert_eq!(executor.source().version, 2);
    }

    #[test]
    fn failed_save_keeps_source_intact_until_force_save_is_explicit() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"old", 1));
        executor.source_mut().force_failure = true;

        assert_eq!(
            executor.edit_file_force_save(command("EDIT /data/note"), b"replacement"),
            Err(Status::NO_SPACE)
        );
        assert_eq!(&executor.source().bytes[..executor.source().len], b"old");
        assert_eq!(executor.source().version, 1);

        executor.source_mut().force_failure = false;
        let metadata = executor
            .edit_file_force_save(command("EDIT /data/note"), b"replacement")
            .expect("explicit retry succeeds");
        assert_eq!(metadata.version, 2);
        assert_eq!(&executor.source().bytes[..executor.source().len], b"replacement");
    }

    #[test]
    fn rmdir_alias_removes_directory_and_reports_removal_metadata() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"", 1));
        let output = executor
            .execute_command(command("RD /data"))
            .expect("RD removes the directory");

        assert_eq!(output.status(), Status::NORMAL);
        assert_eq!(
            output_value(&output, "operation"),
            Some(OutputValue::Text(OutputText::new("removed").unwrap()))
        );
        assert_eq!(
            output_value(&output, "path"),
            Some(OutputValue::Text(OutputText::new("/data").unwrap()))
        );
        assert_eq!(
            output.fields().count(),
            2
        );
    }

    #[test]
    fn rmdir_rejects_the_current_default_directory() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"", 1));
        executor.session.default_directory = Path::new("/data").expect("valid default path");

        assert!(matches!(
            executor.execute_command(command("RMDIR /data")),
            Err(Status::ACCESS_DENIED)
        ));
    }

    #[test]
    fn delete_alias_resolves_relative_path_and_reports_metadata() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"contents", 3));
        executor.session.default_directory = Path::new("/data").expect("valid default path");

        let output = executor
            .execute_command(command("RM \"relative note\""))
            .expect("delete relative file");

        assert_eq!(output.status(), Status::NORMAL);
        assert_eq!(
            output_value(&output, "operation"),
            Some(OutputValue::Text(OutputText::new("deleted").unwrap()))
        );
        assert_eq!(
            output_value(&output, "path"),
            Some(OutputValue::Text(OutputText::new("/data/relative note").unwrap()))
        );
        assert_eq!(output_value(&output, "version"), Some(OutputValue::Unsigned(3)));
        assert_eq!(output_value(&output, "match-count"), Some(OutputValue::Unsigned(1)));
        assert_eq!(
            output_value(&output, "shared-data-reachable"),
            Some(OutputValue::Boolean(false))
        );
    }

    #[test]
    fn delete_preserves_explicit_version_and_rejects_protected_inputs() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"contents", 3));
        let output = executor
            .execute_command(command("DELETE /data/note;1"))
            .expect("delete selected version");
        assert_eq!(output_value(&output, "version"), Some(OutputValue::Unsigned(1)));

        let cases = [
            ("DELETE /", Status::INVALID_PATH),
            ("DELETE /data/note;wat", Status::INVALID_ARGUMENT),
            ("DELETE /data/directory", Status::NOT_DIRECTORY),
        ];
        for (line, status) in cases {
            let mut executor =
                FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"contents", 3));
            assert!(
                matches!(executor.execute_command(command(line)), Err(error) if error == status),
                "{line}"
            );
        }
    }

    #[test]
    fn default_directory_drives_relative_create_type_and_show_workflow() {
        let mut filesystem = MockFilesystem::new(b"contents", 1);
        filesystem.present = false;
        let mut executor = FilesystemExecutor::<_, 16>::new(filesystem);

        executor
            .execute_command(command("MKDIR /data/work"))
            .expect("create workflow directory");
        executor
            .execute_command(command("SET DEFAULT /data"))
            .expect("set workflow default");
        let created = executor
            .execute_command(command("CREATE \"relative file\""))
            .expect("create relative workflow file");
        assert_eq!(
            output_value(&created, "path"),
            Some(OutputValue::Text(OutputText::new("/data/relative file").unwrap()))
        );

        let shown = executor
            .execute_command(command("PWD"))
            .expect("show workflow default");
        assert_eq!(
            output_value(&shown, "default-directory"),
            Some(OutputValue::Text(OutputText::new("/data").unwrap()))
        );

        let restarted = FilesystemExecutor::<_, 16>::new(executor.source);
        assert_eq!(restarted.session().default_directory().as_str(), "/");
    }

    #[test]
    fn wildcard_safety_rules_cover_mutating_and_version_selector_commands() {
        let mut executor = FilesystemExecutor::<_, 16>::new(MockFilesystem::new(b"", 1));
        for line in [
            "CREATE /data/*",
            "MKDIR /data/*",
            "SET DEFAULT /data/*",
            "CD /data/*",
            "LINK /data/* /data/alias",
            "LINK /data/source /data/*",
        ] {
            assert!(
                matches!(
                    executor.execute_command(command(line)),
                    Err(Status::INVALID_ARGUMENT)
                ),
                "{line}"
            );
        }
        assert!(matches!(
            executor.execute_command(command("TYPE /data/*.txt;*")),
            Err(Status::INVALID_ARGUMENT)
        ));
    }
}
