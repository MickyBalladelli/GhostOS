#![no_main]
#![no_std]

use core::arch::asm;
use core::fmt::Write;

use ghostos_abi::{Operation, Request, Response};
use ghostos_shell::editor::{EditorAction, Key, LineEditor};
use ghostos_shell::file_editor::{FileEditor, FileEditorAction};
use ghostos_shell::filesystem::{
    DirectoryEntry, DirectoryPage, DirectoryRemovalMetadata, EntryType, FileMetadata, FileOutput,
    FilesystemSource, Path, ShellSession, MAX_PATH_BYTES,
};
use ghostos_shell::Text;
use ghostos_status::{IntoStatus, Status};

const ABI_VERSION: u16 = ghostos_abi::ABI_SCHEMA_VERSION;
const SHELL_ROLE: u64 = 9;
const REQUIRED_SERVICES: u64 = (1 << 2)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 10)
    | (1 << 13)
    | (1 << 14);
const OPEN_READ: u16 = 1;
const OPEN_WRITE: u16 = 1 << 1;
const OPEN_CREATE: u16 = 1 << 2;
const OPEN_TRUNCATE: u16 = 1 << 3;
const OPEN_EXCLUSIVE: u16 = 1 << 9;
const LIST_PATH_REGION_BYTES: usize = 192;
const LIST_BUFFER_BYTES: usize = 4096;
const EDIT_BYTES: usize = 8192;
const TERMINAL_COLUMNS: usize = 80;
const TERMINAL_ROWS: usize = 24;
const POLL_DELAY_US: u64 = 100;
const HEARTBEAT_PERIOD: u64 = 256;
const HISTORY_CAPACITY: usize = 8;

#[inline(never)]
fn syscall(operation: Operation, flags: u16, capability: u64, arguments: [u64; 6]) -> Response {
    let request = Request {
        operation: operation as u16,
        abi_version: ABI_VERSION,
        flags,
        reserved: 0,
        capability,
        arguments,
    };
    let mut response = Response::EMPTY;
    unsafe {
        asm!(
            "int 0x80",
            in("rdi") &request as *const Request,
            in("rsi") &mut response as *mut Response,
            lateout("rax") _,
            lateout("rcx") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack)
        )
    }
    response
}

fn response_status(response: Response) -> Status {
    Status::from_raw(response.status).unwrap_or(Status::INTERNAL)
}

fn result_status(response: Response) -> Result<Response, Status> {
    let status = response_status(response);
    if status.is_success() {
        Ok(response)
    } else {
        Err(status)
    }
}

fn write_bytes(bytes: &[u8]) {
    let mut offset = 0;
    while offset < bytes.len() {
        let length = core::cmp::min(bytes.len() - offset, 4096);
        let _ = syscall(
            Operation::TerminalWrite,
            0,
            0,
            [bytes.as_ptr() as u64 + offset as u64, length as u64, 0, 0, 0, 0],
        );
        offset += length;
    }
}

fn write_text(text: &str) {
    write_bytes(text.as_bytes())
}

fn read_terminal_byte() -> Option<u8> {
    let mut byte = 0;
    let response = syscall(
        Operation::TerminalRead,
        0,
        0,
        [&mut byte as *mut u8 as u64, 1, 0, 0, 0, 0],
    );
    (response_status(response).is_success() && response.values[0] == 1).then_some(byte)
}

fn read_bridge_byte() -> Option<u8> {
    let mut byte = 0;
    let response = syscall(
        Operation::LoginBridgeRead,
        0,
        0,
        [&mut byte as *mut u8 as u64, 1, 0, 0, 0, 0],
    );
    (response_status(response).is_success() && response.values[0] == 1).then_some(byte)
}

fn sleep_for(duration_us: u64) {
    let now = syscall(Operation::ClockNow, 0, 0, [0; 6]);
    if response_status(now).is_success() {
        let _ = syscall(
            Operation::SleepUntil,
            0,
            0,
            [now.values[0].saturating_add(duration_us), 0, 0, 0, 0, 0],
        );
    }
}

fn service_ready() -> bool {
    response_status(syscall(
        Operation::ServiceReady,
        0,
        0,
        [SHELL_ROLE, 0, 0, 0, 0, 0],
    ))
    .is_success()
}

fn dependencies_ready() -> bool {
    let response = syscall(Operation::SystemInfo, 0, 0, [0; 6]);
    response_status(response).is_success() && response.values[1] & REQUIRED_SERVICES == REQUIRED_SERVICES
}

fn heartbeat(sequence: &mut u64) {
    *sequence = sequence.saturating_add(1);
    let _ = syscall(
        Operation::ServiceHeartbeat,
        0,
        0,
        [SHELL_ROLE, *sequence, 0, 0, 0, 0],
    );
}

fn login_status() -> Option<[u64; 4]> {
    let response = syscall(Operation::LoginStatus, 0, 0, [0; 6]);
    response_status(response).is_success().then_some(response.values)
}

fn fs_call(
    operation: Operation,
    flags: u16,
    capability: u64,
    buffer: Option<&mut [u8]>,
    continuation: u64,
    path_length: u64,
) -> Response {
    let (address, length, writable) = buffer.map_or((0, 0, 0), |buffer| {
        let writable = matches!(operation, Operation::SynFsRead | Operation::SynFsList);
        (buffer.as_mut_ptr() as u64, buffer.len() as u64, writable as u64)
    });
    let arguments = [address, length, writable, 0, continuation, path_length];
    syscall(operation, flags, capability, arguments)
}

fn fs_path_call(operation: Operation, flags: u16, path: &str) -> Result<Response, Status> {
    let bytes = path.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(Status::INVALID_PATH)
    }
    let mut buffer = [0; MAX_PATH_BYTES];
    buffer[..bytes.len()].copy_from_slice(bytes);
    result_status(fs_call(
        operation,
        flags,
        0,
        Some(&mut buffer[..bytes.len()]),
        0,
        0,
    ))
}

fn close_file(capability: u64) -> Result<(), Status> {
    result_status(syscall(Operation::SynFsClose, 0, capability, [0; 6])).map(|_| ())
}

fn file_metadata(capability: u64, path: &str, file_type: EntryType) -> Result<FileMetadata, Status> {
    let response = result_status(syscall(Operation::SynFsMetadata, 0, capability, [0; 6]))?;
    let path = Path::new(path)?;
    Ok(FileMetadata {
        path,
        file_type,
        size: response.values[0],
        version: response.values[1] as u32,
        link_count: 1,
        is_link: file_type == EntryType::Symlink,
    })
}

fn entry_type(raw: u8) -> Option<EntryType> {
    match raw {
        1 => Some(EntryType::File),
        2 => Some(EntryType::Directory),
        3 => Some(EntryType::Symlink),
        _ => None,
    }
}

struct BootFilesystem;

impl BootFilesystem {
    fn open(&mut self, path: &str, flags: u16) -> Result<(u64, FileMetadata), Status> {
        let response = fs_path_call(Operation::SynFsOpen, flags, path)?;
        let capability = response.values[0];
        if capability == 0 {
            return Err(Status::INVALID_ARGUMENT)
        }
        let metadata = file_metadata(capability, path, EntryType::File);
        match metadata {
            Ok(metadata) => Ok((capability, metadata)),
            Err(status) => {
                let _ = close_file(capability);
                Err(status)
            }
        }
    }

    fn lookup_entry(&mut self, path: &str) -> Result<FileMetadata, Status> {
        let (base, selected_version) = ghostos_shell::filesystem::split_version_selector(path)?;
        if base == "/" {
            return Ok(FileMetadata {
                path: Path::ROOT,
                file_type: EntryType::Directory,
                size: 0,
                version: 0,
                link_count: 1,
                is_link: false,
            })
        }
        let (parent, name) = base.rsplit_once('/').ok_or(Status::NOT_FOUND)?;
        let parent = if parent.is_empty() { "/" } else { parent };
        let mut page = DirectoryPage::new();
        let mut continuation = None;
        loop {
            self.list(parent, continuation, &mut page)?;
            for entry in page.entries() {
                if entry.name.as_str() == name
                    && selected_version.map_or(true, |version| version == 0 || version == entry.version)
                {
                    return Ok(FileMetadata {
                        path: Path::new(path)?,
                        file_type: entry.file_type,
                        size: entry.size,
                        version: entry.version,
                        link_count: entry.link_count,
                        is_link: entry.is_link,
                    })
                }
            }
            continuation = page.next.filter(|next| *next != 0);
            if continuation.is_none() {
                return Err(Status::NOT_FOUND)
            }
        }
    }

    fn list_page(&mut self, path: &str, continuation: u64, output: &mut DirectoryPage) -> Result<(), Status> {
        let bytes = path.as_bytes();
        if bytes.is_empty() || bytes.len() > LIST_PATH_REGION_BYTES {
            return Err(Status::INVALID_PATH)
        }
        let mut buffer = [0; LIST_BUFFER_BYTES];
        buffer[..bytes.len()].copy_from_slice(bytes);
        let response = result_status(fs_call(
            Operation::SynFsList,
            0,
            0,
            Some(&mut buffer),
            continuation,
            bytes.len() as u64,
        ))?;
        let length = response.values[0] as usize;
        if length > LIST_BUFFER_BYTES - LIST_PATH_REGION_BYTES {
            return Err(Status::CORRUPT)
        }
        let next = u32::try_from(response.values[1]).map_err(|_| Status::CORRUPT)?;
        output.clear();
        let mut offset = LIST_PATH_REGION_BYTES;
        let end = offset + length;
        while offset < end {
            if offset + 22 > end {
                return Err(Status::CORRUPT)
            }
            let name_length = u16::from_le_bytes([buffer[offset], buffer[offset + 1]]) as usize;
            let record_length = 22usize.checked_add(name_length).ok_or(Status::CORRUPT)?;
            if offset + record_length > end {
                return Err(Status::CORRUPT)
            }
            let file_type = entry_type(buffer[offset + 2]).ok_or(Status::CORRUPT)?;
            let size = u64::from_le_bytes(buffer[offset + 4..offset + 12].try_into().unwrap());
            let version = u32::from_le_bytes(buffer[offset + 12..offset + 16].try_into().unwrap());
            let link_count = u32::from_le_bytes(buffer[offset + 16..offset + 20].try_into().unwrap());
            let name = core::str::from_utf8(&buffer[offset + 22..offset + record_length])
                .map_err(|_| Status::CORRUPT)?;
            output.push(DirectoryEntry {
                name: Path::new(name)?,
                file_type,
                size,
                version,
                link_count,
                is_link: file_type == EntryType::Symlink,
            })?;
            offset += record_length;
        }
        output.next = (next != 0).then_some(next);
        Ok(())
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
        let (capability, _) = self.open(path, OPEN_WRITE | OPEN_CREATE | OPEN_TRUNCATE)?;
        let result = if contents.is_empty() {
            Ok(())
        } else {
            let mut bytes = [0; EDIT_BYTES];
            if contents.len() > bytes.len() {
                let _ = close_file(capability);
                return Err(Status::NO_SPACE)
            }
            bytes[..contents.len()].copy_from_slice(contents);
            result_status(fs_call(
                Operation::SynFsWrite,
                0,
                capability,
                Some(&mut bytes[..contents.len()]),
                0,
                0,
            ))
            .map(|_| ())
        };
        let close = close_file(capability);
        result?;
        close?;
        self.lookup_entry(path)
    }
}

impl FilesystemSource for BootFilesystem {
    fn directory_exists(&mut self, path: &str) -> Result<bool, Status> {
        let mut page = DirectoryPage::new();
        match self.list(path, None, &mut page) {
            Ok(()) => Ok(true),
            Err(Status::NOT_FOUND | Status::NOT_DIRECTORY) => Ok(false),
            Err(status) => Err(status),
        }
    }

    fn list(
        &mut self,
        path: &str,
        continuation: Option<u32>,
        output: &mut DirectoryPage,
    ) -> Result<(), Status> {
        self.list_page(path, continuation.unwrap_or(0) as u64, output)
    }

    fn create_directory(&mut self, path: &str, recursive: bool) -> Result<FileMetadata, Status> {
        let response = fs_path_call(
            Operation::SynFsMkdir,
            if recursive { 1 << 8 } else { 0 },
            path,
        )?;
        Ok(FileMetadata {
            path: Path::new(path)?,
            file_type: EntryType::Directory,
            size: response.values[0],
            version: response.values[1] as u32,
            link_count: 1,
            is_link: false,
        })
    }

    fn create_file(&mut self, path: &str) -> Result<FileMetadata, Status> {
        let (capability, metadata) = self.open(
            path,
            OPEN_READ | OPEN_WRITE | OPEN_CREATE | OPEN_EXCLUSIVE,
        )?;
        let result = file_metadata(capability, path, EntryType::File).or(Ok(metadata));
        let close = close_file(capability);
        result.and_then(|metadata| close.map(|_| metadata))
    }

    fn remove_directory(&mut self, path: &str) -> Result<DirectoryRemovalMetadata, Status> {
        let response = fs_path_call(Operation::SynFsRmdir, 0, path)?;
        Ok(DirectoryRemovalMetadata {
            directory: FileMetadata {
                path: Path::new(path)?,
                file_type: EntryType::Directory,
                size: response.values[0],
                version: response.values[1] as u32,
                link_count: 1,
                is_link: false,
            },
            removal_generation: response.values[2],
            storage_reclamation_pending: response.values[3] != 0,
        })
    }

    fn delete(&mut self, path: &str) -> Result<ghostos_shell::filesystem::DeleteMetadata, Status> {
        let response = fs_path_call(Operation::SynFsDelete, 0, path)?;
        let file_type = entry_type(response.values[1] as u8).ok_or(Status::CORRUPT)?;
        Ok(ghostos_shell::filesystem::DeleteMetadata {
            file: FileMetadata {
                path: Path::new(path)?,
                file_type,
                size: 0,
                version: response.values[0] as u32,
                link_count: response.values[2] as u32,
                is_link: file_type == EntryType::Symlink,
            },
            shared_data_reachable: response.values[3] != 0,
        })
    }

    fn type_file(
        &mut self,
        path: &str,
        _binary: bool,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        let listed = self.lookup_entry(path).ok();
        let (capability, mut metadata) = self.open(path, OPEN_READ)?;
        if let Some(listed) = listed {
            metadata.file_type = listed.file_type;
            metadata.is_link = listed.is_link;
            metadata.link_count = listed.link_count;
        }
        let mut buffer = [0; 4096];
        let mut offset = 0;
        let mut result = Ok(());
        loop {
            let response = fs_call(
                Operation::SynFsRead,
                0,
                capability,
                Some(&mut buffer),
                offset,
                0,
            );
            match result_status(response) {
                Ok(response) if response.values[0] != 0 => {
                    let length = response.values[0] as usize;
                    if let Err(status) = output.write(&buffer[..length]) {
                        result = Err(status);
                        break
                    }
                    offset = offset.saturating_add(length as u64)
                }
                Ok(_) => break,
                Err(status) => {
                    result = Err(status);
                    break
                }
            }
        }
        let close = close_file(capability);
        result?;
        close?;
        metadata.size = offset;
        Ok(metadata)
    }

    fn save_file(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
        self.write_file(path, contents)
    }

    fn save_file_if_version(
        &mut self,
        path: &str,
        expected_version: u32,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        if let Ok(metadata) = self.lookup_entry(path)
            && expected_version != 0
            && metadata.version != expected_version
        {
            return Err(Status::CONFLICT)
        }
        self.write_file(path, contents)
    }

    fn save_file_force(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
        self.write_file(path, contents)
    }
}

struct OutputSink {
    bytes: [u8; EDIT_BYTES],
    len: usize,
}

impl OutputSink {
    const fn new() -> Self {
        Self {
            bytes: [0; EDIT_BYTES],
            len: 0,
        }
    }
}

impl FileOutput for OutputSink {
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

struct VtInput {
    state: VtState,
    pending: Option<u8>,
    parameter: u16,
    modifier: u16,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_expected: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VtState {
    Ground,
    Escape,
    Csi,
    Ss3,
}

impl VtInput {
    const fn new() -> Self {
        Self {
            state: VtState::Ground,
            pending: None,
            parameter: 0,
            modifier: 0,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_expected: 0,
        }
    }

    fn advance(&mut self, byte: u8) -> Option<Key> {
        if let Some(pending) = self.pending.take() {
            self.pending = Some(byte);
            return self.advance(pending)
        }
        if self.state == VtState::Ground && self.utf8_expected != 0 {
            if byte & 0xc0 != 0x80 || self.utf8_len == self.utf8.len() {
                self.utf8_len = 0;
                self.utf8_expected = 0;
                return None
            }
            self.utf8[self.utf8_len] = byte;
            self.utf8_len += 1;
            if self.utf8_len == self.utf8_expected {
                let key = core::str::from_utf8(&self.utf8[..self.utf8_len])
                    .ok()
                    .and_then(|text| text.chars().next())
                    .map(Key::Character);
                self.utf8_len = 0;
                self.utf8_expected = 0;
                return key
            }
            return None
        }
        match self.state {
            VtState::Ground => match byte {
                0x1b => {
                    self.state = VtState::Escape;
                    None
                }
                b'\r' | b'\n' => Some(Key::Enter),
                b'\t' => Some(Key::Tab),
                8 | 127 => Some(Key::Backspace),
                3 => Some(Key::Cancel),
                19 => Some(Key::Save),
                24 => Some(Key::DiscardExit),
                26 => Some(Key::SaveExit),
                0xc2..=0xdf => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 2;
                    None
                }
                0xe0..=0xef => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 3;
                    None
                }
                0xf0..=0xf4 => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 4;
                    None
                }
                0x20..=0x7e => Some(Key::Character(byte as char)),
                _ => None,
            },
            VtState::Escape => {
                self.parameter = 0;
                self.modifier = 0;
                match byte {
                    b'[' => {
                        self.state = VtState::Csi;
                        None
                    }
                    b'O' => {
                        self.state = VtState::Ss3;
                        None
                    }
                    _ => {
                        self.state = VtState::Ground;
                        self.pending = Some(byte);
                        Some(Key::Escape)
                    }
                }
            }
            VtState::Csi => match byte {
                b'0'..=b'9' => {
                    if self.modifier == 0 {
                        self.parameter = self.parameter.saturating_mul(10).saturating_add((byte - b'0') as u16)
                    } else {
                        self.modifier = self.modifier.saturating_mul(10).saturating_add((byte - b'0') as u16)
                    }
                    None
                }
                b';' => {
                    self.modifier = 1;
                    None
                }
                _ => {
                    self.state = VtState::Ground;
                    self.navigation_key(byte)
                }
            },
            VtState::Ss3 => {
                self.state = VtState::Ground;
                self.navigation_key(byte)
            }
        }
    }

    fn navigation_key(&self, byte: u8) -> Option<Key> {
        let shifted = self.modifier == 2;
        match byte {
            b'A' => Some(if shifted { Key::ShiftUp } else { Key::Up }),
            b'B' => Some(if shifted { Key::ShiftDown } else { Key::Down }),
            b'C' => Some(if shifted { Key::ShiftRight } else { Key::Right }),
            b'D' => Some(if shifted { Key::ShiftLeft } else { Key::Left }),
            b'H' => Some(if shifted { Key::ShiftHome } else { Key::Home }),
            b'F' => Some(if shifted { Key::ShiftEnd } else { Key::End }),
            b'~' => match self.parameter {
                1 | 7 => Some(if shifted { Key::ShiftHome } else { Key::Home }),
                3 => Some(Key::Delete),
                4 | 8 => Some(if shifted { Key::ShiftEnd } else { Key::End }),
                5 => Some(Key::PageUp),
                6 => Some(Key::PageDown),
                _ => None,
            },
            _ => None,
        }
    }
}

fn prompt(editor: &LineEditor<HISTORY_CAPACITY>) {
    let mut output = Text::<2048>::empty();
    let _ = write!(output, "\r\x1b[2K$ {}", editor.line());
    let after = editor.line()[editor.cursor()..].chars().count();
    if after != 0 {
        let _ = write!(output, "\x1b[{}D", after);
    }
    write_bytes(output.as_str().as_bytes())
}

fn print_status(status: Status) {
    write_text("Reason: ");
    write_text(status.message());
    write_text("\n");
}

fn command_name(line: &str) -> &str {
    line.split_ascii_whitespace().next().unwrap_or("")
}

fn is_command(line: &str, name: &str) -> bool {
    command_name(line).eq_ignore_ascii_case(name)
}

fn print_help() {
    write_text("GhostOS Rust shell\n");
    write_text("  HELP, ?, COMMANDS       Show this list\n");
    write_text("  DIRECTORY|DIR|LS [path] List a directory\n");
    write_text("  CD|CHDIR [path]         Change directory\n");
    write_text("  PWD                     Print working directory\n");
    write_text("  CREATE <path>           Create a file\n");
    write_text("  TYPE|CAT <path>         Read a file\n");
    write_text("  EDIT <path>             Edit a UTF-8 file\n");
    write_text("  MKDIR <path>            Create a directory\n");
    write_text("  RMDIR|RD <path>         Remove a directory\n");
    write_text("  DELETE|DEL <path>       Delete a file\n");
    write_text("  WHOAMI                  Show logged-in user\n");
    write_text("  LOGIN                   Start login\n");
    write_text("  LOGOUT                  Lock the shell\n");
    write_text("  SHUTDOWN                Power off\n");
    write_text("\nEDIT keys: Ctrl-S save, Ctrl-Z save and exit, Ctrl-X discard, Esc command mode\n");
}

fn execute_simple(line: &str) -> bool {
    if is_command(line, "HELP") || is_command(line, "?") || is_command(line, "COMMANDS") {
        print_help();
        return true
    }
    if is_command(line, "SHUTDOWN") {
        let _ = syscall(Operation::Shutdown, 0, 0, [0; 6]);
        return true
    }
    if is_command(line, "LOGIN") {
        let response = syscall(Operation::LoginStart, 0, 0, [0; 6]);
        if response_status(response).is_success() {
            write_text("Starting login...\n");
        } else {
            write_text("Login request failed\n");
            print_status(response_status(response));
        }
        return true
    }
    if is_command(line, "LOGOUT") {
        let response = syscall(Operation::LoginLogout, 0, 0, [0; 6]);
        if response_status(response).is_success() {
            write_text("Logging out...\n");
        } else {
            write_text("Logout request failed\n");
            print_status(response_status(response));
        }
        return true
    }
    if is_command(line, "WHOAMI") {
        let mut username = [0; 128];
        let response = syscall(
            Operation::LoginWhoami,
            0,
            0,
            [&mut username as *mut u8 as u64, username.len() as u64, 0, 0, 0, 0],
        );
        if let Ok(response) = result_status(response) {
            let length = response.values[0] as usize;
            if length <= username.len() {
                write_bytes(&username[..length]);
                write_text("\n");
                return true
            }
        }
        write_text("Whoami request failed\n");
        print_status(response_status(response));
        return true
    }
    false
}

fn edit_file(
    filesystem: &mut BootFilesystem,
    session: &ShellSession,
    requested: Option<&str>,
) {
    let path = match session.resolve(requested) {
        Ok(path) => path,
        Err(status) => {
            print_status(status);
            return
        }
    };
    let mut sink = OutputSink::new();
    let metadata = match filesystem.type_file(path.as_str(), false, &mut sink) {
        Ok(metadata) => metadata,
        Err(Status::NOT_FOUND) => match filesystem.create_file(path.as_str()) {
            Ok(metadata) => metadata,
            Err(status) => {
                print_status(status);
                return
            }
        },
        Err(status) => {
            print_status(status);
            return
        }
    };
    let mut editor = match FileEditor::<EDIT_BYTES>::new(
        path.as_str(),
        metadata.version,
        &sink.bytes[..sink.len],
    ) {
        Ok(editor) => editor,
        Err(error) => {
            print_status(error.status());
            return
        }
    };
    let mut input = VtInput::new();
    write_text("\x1b[2J\x1b[H");
    redraw_file_editor(&mut editor);
    let mut discard_prompt = false;
    loop {
        let Some(byte) = read_terminal_byte() else {
            sleep_for(POLL_DELAY_US);
            continue
        };
        let Some(key) = input.advance(byte) else {
            continue
        };
        if discard_prompt {
            discard_prompt = false;
            match key {
                Key::Character('y') | Key::Character('Y') => break,
                _ => {
                    redraw_file_editor(&mut editor);
                    continue
                }
            }
        }
        let action = match editor.handle(key) {
            Ok(action) => action,
            Err(error) => {
                print_status(error.status());
                continue
            }
        };
        match action {
            FileEditorAction::None => {}
            FileEditorAction::Redraw => redraw_file_editor(&mut editor),
            FileEditorAction::PromptDiscard => {
                write_text("\r\x1b[2KDiscard changes? [y/N] ");
                discard_prompt = true;
            }
            FileEditorAction::DiscardExit => break,
            FileEditorAction::Save | FileEditorAction::SaveExit => {
                let exit = action == FileEditorAction::SaveExit;
                match filesystem.save_file_if_version(
                    path.as_str(),
                    editor.version(),
                    editor.bytes(),
                ) {
                    Ok(metadata) => {
                        editor.mark_saved(metadata.version);
                        if exit {
                            break
                        }
                        redraw_file_editor(&mut editor)
                    }
                    Err(Status::CONFLICT) => {
                        write_text("\nSave conflict. File changed; edits kept in memory.\n");
                        redraw_file_editor(&mut editor)
                    }
                    Err(status) => {
                        write_text("\nSave failed. ");
                        print_status(status);
                        redraw_file_editor(&mut editor)
                    }
                }
            }
        }
    }
    write_text("\x1b[2J\x1b[H");
}

fn redraw_file_editor(editor: &mut FileEditor<EDIT_BYTES>) {
    match editor.render::<8192>(TERMINAL_COLUMNS, TERMINAL_ROWS) {
        Ok(text) => write_text(text.as_str()),
        Err(error) => print_status(error.status()),
    }
}

fn list_directory(
    filesystem: &mut BootFilesystem,
    session: &ShellSession,
    requested: Option<&str>,
) {
    let path = match session.resolve(requested) {
        Ok(path) => path,
        Err(status) => {
            print_status(status);
            return
        }
    };
    let mut continuation = None;
    loop {
        let mut page = DirectoryPage::new();
        if let Err(status) = filesystem.list(path.as_str(), continuation, &mut page) {
            print_status(status);
            return
        }
        for entry in page.entries() {
            write_text(entry.name.as_str());
            write_text("\n");
        }
        continuation = page.next;
        if continuation.is_none() {
            return
        }
    }
}

fn execute_line(line: &str, filesystem: &mut BootFilesystem, session: &mut ShellSession) {
    if execute_simple(line) {
        return
    }
    let mut words = line.split_ascii_whitespace();
    let Some(name) = words.next() else {
        return
    };
    let first = words.next();
    if words.next().is_some() {
        write_text("Too many arguments.\n");
        return
    }
    if name.eq_ignore_ascii_case("PWD") {
        if first.is_some() {
            write_text("Too many arguments.\n");
        } else {
            write_text(session.default_directory().as_str());
            write_text("\n");
        }
        return
    }
    if name.eq_ignore_ascii_case("LS")
        || name.eq_ignore_ascii_case("DIR")
        || name.eq_ignore_ascii_case("DIRECTORY")
    {
        list_directory(filesystem, session, first);
        return
    }
    if name.eq_ignore_ascii_case("CD") || name.eq_ignore_ascii_case("CHDIR") {
        match session.set_default(filesystem, first.unwrap_or(".")) {
            Ok(_) => {}
            Err(status) => print_status(status),
        }
        return
    }
    let Some(requested) = first else {
        write_text("Path required.\n");
        return
    };
    let path = match session.resolve(Some(requested)) {
        Ok(path) => path,
        Err(status) => {
            print_status(status);
            return
        }
    };
    if name.eq_ignore_ascii_case("CREATE") {
        match filesystem.create_file(path.as_str()) {
            Ok(_) => write_text("created\n"),
            Err(status) => print_status(status),
        }
    } else if name.eq_ignore_ascii_case("TYPE") || name.eq_ignore_ascii_case("CAT") {
        let mut output = OutputSink::new();
        match filesystem.type_file(path.as_str(), false, &mut output) {
            Ok(_) => {
                write_bytes(&output.bytes[..output.len]);
                write_text("\n");
            }
            Err(status) => print_status(status),
        }
    } else if name.eq_ignore_ascii_case("MKDIR") {
        match filesystem.create_directory(path.as_str(), false) {
            Ok(_) => write_text("created\n"),
            Err(status) => print_status(status),
        }
    } else if name.eq_ignore_ascii_case("RMDIR") || name.eq_ignore_ascii_case("RD") {
        match filesystem.remove_directory(path.as_str()) {
            Ok(_) => write_text("removed\n"),
            Err(status) => print_status(status),
        }
    } else if name.eq_ignore_ascii_case("DELETE") || name.eq_ignore_ascii_case("DEL") {
        match filesystem.delete(path.as_str()) {
            Ok(_) => write_text("deleted\n"),
            Err(status) => print_status(status),
        }
    } else if name.eq_ignore_ascii_case("EDIT") {
        edit_file(filesystem, session, Some(requested));
    } else {
        write_text("Unknown command. Use HELP.\n");
    }
}

fn bridge_read() -> Option<u8> {
    loop {
        if let Some(byte) = read_bridge_byte() {
            return Some(byte)
        }
        sleep_for(POLL_DELAY_US);
    }
}

fn bridge_line(output: &mut [u8]) -> usize {
    let first = bridge_read().unwrap_or(0);
    if first == 0 {
        let low = bridge_read().unwrap_or(0) as usize;
        let high = bridge_read().unwrap_or(0) as usize;
        let length = low | (high << 8);
        if length == 0 || length >= output.len() {
            return 0
        }
        for byte in output.iter_mut().take(length) {
            *byte = bridge_read().unwrap_or(0)
        }
        return length
    }
    let mut length = 0;
    let mut byte = first;
    loop {
        if byte == b'\r' || byte == b'\n' {
            return length
        }
        if byte >= 32 && byte < 127 && length + 1 < output.len() {
            output[length] = byte;
            length += 1;
        }
        byte = bridge_read().unwrap_or(0);
    }
}

fn credential_kind(answer: &[u8]) -> u64 {
    if answer.is_empty() || answer.eq_ignore_ascii_case(b"PASSKEY") || answer == b"1" {
        1
    } else if answer.eq_ignore_ascii_case(b"TPM") || answer == b"2" {
        2
    } else if answer.eq_ignore_ascii_case(b"SSH") || answer == b"3" {
        3
    } else {
        0
    }
}

fn canonical_passkey(material: &[u8]) -> bool {
    material.len() == 77
        && material[..10] == [0xa5, 1, 2, 3, 0x26, 0x20, 1, 0x21, 0x58, 0x20]
        && material[42..45] == [0x22, 0x58, 0x20]
}

fn reset_first_admin_staging() -> bool {
    let clear = syscall(
        Operation::LoginBootstrapRecovery,
        0,
        0,
        [2, 0, 0, 0, 0, 0],
    );
    if !response_status(clear).is_success() {
        return false
    }
    let clear = syscall(
        Operation::LoginBootstrapRecovery,
        0,
        0,
        [1, 0, 0, 0, 0, 0],
    );
    response_status(clear).is_success() && clear.values[0] == 0 && clear.values[1] == 0
}

fn run_first_run_wizard() {
    let mut username = [0; 64];
    let mut kind = [0; 32];
    let mut material = [0; 96];
    let mut answer = [0; 32];
    write_text("No administrator account exists.\n");
    write_text("GhostOS first-run setup mode\n");
    write_text("Open the local passkey URL shown by the VM host for guided setup.\n");
    write_text("Passkey public material arrives through the local bridge and stays hidden.\n");
    loop {
        write_text("\n\x1b]GhostOSEnroll\x07");
        let username_length = bridge_line(&mut username);
        if !(1..=32).contains(&username_length) {
            write_text("Username must be 1-32 valid characters.\n");
            continue
        }
        if !reset_first_admin_staging() {
            write_text("Previous setup state could not be cleared.\n");
            continue
        }
        write_text("Credential type [PASSKEY/TPM/SSH] (PASSKEY): ");
        let kind_length = bridge_line(&mut kind);
        let kind_id = credential_kind(&kind[..kind_length]);
        if kind_id == 0 {
            write_text("Unknown credential type. Use PASSKEY, TPM, or SSH.\n");
            continue
        }
        write_text("Waiting for passkey public key from local browser: ");
        let material_length = bridge_line(&mut material);
        let stored_length = if kind_id == 1 && canonical_passkey(&material[..material_length]) {
            material_length
        } else if kind_id != 1 {
            material_length
        } else {
            0
        };
        if stored_length == 0 {
            write_text("Credential material is invalid.\n");
            continue
        }
        write_text("\nUsername: ");
        write_bytes(&username[..username_length]);
        write_text("\nCredential type: ");
        write_text(match kind_id {
            2 => "TPM",
            3 => "SSH",
            _ => "PASSKEY",
        });
        write_text("\nMaterial: received\nCreate this administrator account? [y/N]: ");
        let answer_length = bridge_line(&mut answer);
        if answer[..answer_length].eq_ignore_ascii_case(b"SHUTDOWN") {
            let _ = syscall(Operation::Shutdown, 0, 0, [0; 6]);
            continue
        }
        if !answer[..answer_length].eq_ignore_ascii_case(b"Y")
            && !answer[..answer_length].eq_ignore_ascii_case(b"YES")
        {
            write_text("Setup restarted with new answers.\n");
            continue
        }
        let username_response = syscall(
            Operation::LoginBootstrapUsername,
            0,
            0,
            [username.as_ptr() as u64, username_length as u64, 0, 0, 0, 0],
        );
        if !response_status(username_response).is_success() {
            write_text("Username rejected. Repeat setup.\n");
            continue
        }
        let credential_response = syscall(
            Operation::LoginBootstrapCredential,
            0,
            0,
            [kind_id, material.as_ptr() as u64, stored_length as u64, 0, 0, 0],
        );
        if !response_status(credential_response).is_success() {
            write_text("Credential rejected. Repeat setup.\n");
            continue
        }
        let confirm = syscall(Operation::LoginBootstrapConfirm, 0, 0, [0; 6]);
        if !response_status(confirm).is_success() {
            write_text("Confirmation rejected. Repeat setup.\n");
            continue
        }
        write_text("Administrator account committed.\n");
        return
    }
}

fn shell_loop() -> ! {
    let mut filesystem = BootFilesystem;
    let mut session = ShellSession::new();
    let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
    let mut input = VtInput::new();
    let mut heartbeat_sequence = 0;
    let mut idle_polls = 0;
    let mut prompted = false;
    write_text("GhostOS user shell\n");
    loop {
        let Some(status) = login_status() else {
            sleep_for(POLL_DELAY_US);
            continue
        };
        if status[1] == 0 {
            if !prompted {
                run_first_run_wizard();
                prompted = true;
            }
        }
        if status[3] == 0 {
            prompted = false;
            sleep_for(POLL_DELAY_US);
            idle_polls += 1;
            if idle_polls >= HEARTBEAT_PERIOD {
                heartbeat(&mut heartbeat_sequence);
                idle_polls = 0;
            }
            continue
        }
        if !prompted {
            write_text("\n$ ");
            prompted = true;
        }
        let Some(byte) = read_terminal_byte() else {
            sleep_for(POLL_DELAY_US);
            idle_polls += 1;
            if idle_polls >= HEARTBEAT_PERIOD {
                heartbeat(&mut heartbeat_sequence);
                idle_polls = 0;
            }
            continue
        };
        idle_polls = 0;
        let Some(key) = input.advance(byte) else {
            continue
        };
        match editor.handle(key) {
            Ok(EditorAction::Redraw) => prompt(&editor),
            Ok(EditorAction::Complete) => prompt(&editor),
            Ok(EditorAction::Submit(line)) => {
                write_text("\n");
                if !line.as_str().trim().is_empty() {
                    execute_line(line.as_str(), &mut filesystem, &mut session)
                }
                prompt(&editor);
            }
            Ok(EditorAction::Cancel) => {
                write_text("\n^C\n$ ");
            }
            Ok(EditorAction::None) => {}
            Err(error) => {
                write_text("\nShell input error. ");
                print_status(error.status());
                editor.clear();
                prompt(&editor);
            }
        }
    }
}

#[unsafe(export_name = "_start")]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn start() -> ! {
    while !dependencies_ready() {
        sleep_for(POLL_DELAY_US);
    }
    while !service_ready() {
        sleep_for(POLL_DELAY_US);
    }
    shell_loop()
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop()
    }
}
