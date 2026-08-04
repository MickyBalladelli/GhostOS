use std::vec::Vec;

use synos_status::Status;
use synos_webterm::{
    AuthenticatedPrincipal, ShellBackend, SshAuthenticator, SshDaemon, SshError, Terminal,
    TerminalSize,
};
use syn_shell::filesystem::{
    register_filesystem_commands, DirectoryPage, DirectoryRemovalMetadata, EntryType,
    FileMetadata, FileOutput, FilesystemExecutor, FilesystemSource, Path,
};
use syn_shell::parser::CommandRegistry;

struct Auth;

impl SshAuthenticator for Auth {
    fn authenticate(
        &mut self,
        username: &str,
        _public_key: &[u8],
        _signature: &[u8],
        _exchange_hash: &[u8],
    ) -> Result<AuthenticatedPrincipal, SshError> {
        if username == "user" {
            Ok(AuthenticatedPrincipal { identity: 7, shell_capability: 9 })
        } else {
            Err(SshError::AuthenticationFailed)
        }
    }
}

struct Shell {
    input: Vec<u8>,
    resized: Option<TerminalSize>,
    closed: bool,
}

impl ShellBackend for Shell {
    type Handle = u64;

    fn open(&mut self, _principal: AuthenticatedPrincipal, _terminal: TerminalSize) -> Result<Self::Handle, SshError> {
        Ok(1)
    }

    fn input(&mut self, _handle: Self::Handle, bytes: &[u8]) -> Result<usize, SshError> {
        self.input.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn resize(&mut self, _handle: Self::Handle, terminal: TerminalSize) -> Result<(), SshError> {
        self.resized = Some(terminal);
        Ok(())
    }

    fn output(&mut self, _handle: Self::Handle, bytes: &mut [u8]) -> Result<usize, SshError> {
        let output = b"ready";
        let count = output.len().min(bytes.len());
        bytes[..count].copy_from_slice(&output[..count]);
        Ok(count)
    }

    fn close(&mut self, _handle: Self::Handle) {
        self.closed = true;
    }
}

#[test]
fn ssh_session_auth_resize_io_and_stale_handle() {
    let terminal = TerminalSize::new(80, 24).unwrap();
    let mut daemon = SshDaemon::<_, _, 1>::new(
        Auth,
        Shell { input: Vec::new(), resized: None, closed: false },
    );
    assert_eq!(daemon.open_public_key_session("bad", b"key", b"sig", b"hash", terminal), Err(SshError::AuthenticationFailed));
    let session = daemon.open_public_key_session("user", b"key", b"sig", b"hash", terminal).unwrap();
    assert_eq!(session.raw() & u32::MAX as u64, 0);
    assert_eq!(daemon.input(session, b"show").unwrap(), 4);
    daemon.resize(session, TerminalSize::new(100, 40).unwrap()).unwrap();
    let mut output = [0; 5];
    assert_eq!(daemon.output(session, &mut output).unwrap(), 5);
    assert_eq!(&output, b"ready");
    daemon.close(session).unwrap();
    assert_eq!(daemon.input(session, b"x"), Err(SshError::InvalidSession));
}

#[test]
fn terminal_decodes_utf8_escape_and_dirty_rows() {
    let mut terminal = Terminal::<4, 2>::new().unwrap();
    terminal.mark_row_clean(0);
    terminal.write("A\u{00e9}".as_bytes());
    assert_eq!(terminal.row(0).unwrap()[0].glyph, 'A' as u32);
    assert_eq!(terminal.row(0).unwrap()[1].glyph, 0xe9);
    terminal.mark_row_clean(0);
    assert!(!terminal.row_is_dirty(0));
    terminal.write(b"\x1b[2J");
    assert!(terminal.row_is_dirty(0));
    assert_eq!(TerminalSize::new(0, 24), None);
}

#[derive(Clone, Copy)]
struct DirectoryFixture {
    path: &'static str,
    exists: bool,
    has_child: bool,
    protected: bool,
}

struct RemoteFilesystem {
    directories: [DirectoryFixture; 3],
}

impl RemoteFilesystem {
    fn new() -> Self {
        Self {
            directories: [
                DirectoryFixture {
                    path: "/empty",
                    exists: true,
                    has_child: false,
                    protected: false,
                },
                DirectoryFixture {
                    path: "/non-empty",
                    exists: true,
                    has_child: true,
                    protected: false,
                },
                DirectoryFixture {
                    path: "/protected",
                    exists: true,
                    has_child: false,
                    protected: true,
                },
            ],
        }
    }

    fn metadata(path: &str) -> FileMetadata {
        FileMetadata {
            path: Path::new(path).expect("fixture path"),
            file_type: EntryType::Directory,
            size: 0,
            version: 1,
            link_count: 1,
            is_link: false,
        }
    }
}

impl FilesystemSource for RemoteFilesystem {
    fn directory_exists(&mut self, path: &str) -> Result<bool, Status> {
        Ok(path == "/" || self.directories.iter().any(|directory| {
            directory.path == path && directory.exists
        }))
    }

    fn list(
        &mut self,
        _path: &str,
        _continuation: Option<u32>,
        _output: &mut DirectoryPage,
    ) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }

    fn create_directory(&mut self, _path: &str, _recursive: bool) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }

    fn create_file(&mut self, _path: &str) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }

    fn remove_directory(&mut self, path: &str) -> Result<DirectoryRemovalMetadata, Status> {
        let directory = self
            .directories
            .iter_mut()
            .find(|directory| directory.path == path && directory.exists)
            .ok_or(Status::NOT_FOUND)?;
        if directory.protected {
            return Err(Status::ACCESS_DENIED)
        }
        if directory.has_child {
            return Err(Status::DIRECTORY_NOT_EMPTY)
        }
        directory.exists = false;
        Ok(DirectoryRemovalMetadata {
            directory: Self::metadata(path),
            removal_generation: 2,
            storage_reclamation_pending: false,
        })
    }

    fn type_file(
        &mut self,
        _path: &str,
        _binary: bool,
        _output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        Err(Status::NOT_FOUND)
    }
}

struct RmdirShell {
    input: Vec<u8>,
    output: Vec<u8>,
    registry: CommandRegistry<16>,
    executor: FilesystemExecutor<RemoteFilesystem>,
    resized: Option<TerminalSize>,
    closed: bool,
}

impl RmdirShell {
    fn new() -> Self {
        let mut registry = CommandRegistry::<16>::new();
        register_filesystem_commands(&mut registry).expect("filesystem commands");
        Self {
            input: Vec::new(),
            output: Vec::new(),
            registry,
            executor: FilesystemExecutor::new(RemoteFilesystem::new()),
            resized: None,
            closed: false,
        }
    }

    fn process_lines(&mut self) -> Result<(), SshError> {
        while let Some(index) = self.input.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.input.drain(..=index).collect();
            let line = &line[..line.len() - 1];
            let line = core::str::from_utf8(line).map_err(|_| SshError::WouldBlock)?;
            let result = self
                .registry
                .parse(line)
                .ok()
                .and_then(|program| program.stage(0))
                .ok_or(Status::INVALID_ARGUMENT)
                .and_then(|command| self.executor.execute_command(command));
            match result {
                Ok(_) => self.output.extend_from_slice(b"ok\n"),
                Err(status) => {
                    self.output.extend_from_slice(status.message().as_bytes());
                    self.output.push(b'\n');
                }
            }
        }
        Ok(())
    }
}

impl ShellBackend for RmdirShell {
    type Handle = u64;

    fn open(
        &mut self,
        _principal: AuthenticatedPrincipal,
        _terminal: TerminalSize,
    ) -> Result<Self::Handle, SshError> {
        Ok(1)
    }

    fn input(&mut self, _handle: Self::Handle, bytes: &[u8]) -> Result<usize, SshError> {
        self.input.extend_from_slice(bytes);
        self.process_lines()?;
        Ok(bytes.len())
    }

    fn resize(&mut self, _handle: Self::Handle, terminal: TerminalSize) -> Result<(), SshError> {
        self.resized = Some(terminal);
        Ok(())
    }

    fn output(&mut self, _handle: Self::Handle, bytes: &mut [u8]) -> Result<usize, SshError> {
        let count = bytes.len().min(self.output.len());
        bytes[..count].copy_from_slice(&self.output[..count]);
        self.output.drain(..count);
        Ok(count)
    }

    fn close(&mut self, _handle: Self::Handle) {
        self.closed = true;
    }
}

#[test]
fn remote_terminal_rmdir_keeps_non_empty_and_protected_directories_safe() {
    let terminal = TerminalSize::new(80, 24).unwrap();
    let mut daemon = SshDaemon::<_, _, 1>::new(Auth, RmdirShell::new());
    let session = daemon
        .open_public_key_session("user", b"key", b"sig", b"hash", terminal)
        .expect("remote terminal session opens");

    let commands = b"rmdir /empty\nrmdir /non-empty\nrmdir /protected\nset default /protected\nrmdir /protected\nrmdir /\nrmdir /empty\nrmdir /non-empty\n";
    assert_eq!(daemon.input(session, commands), Ok(commands.len()));

    let mut output = [0; 256];
    let count = daemon.output(session, &mut output).expect("read remote output");
    assert_eq!(
        &output[..count],
        b"ok\ndirectory not empty\naccess denied\nok\naccess denied\ninvalid path\npath not found\ndirectory not empty\n"
    );
}
