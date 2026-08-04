use std::vec::Vec;

use synos_status::Status;
use syn_shell::editor::Key;
use syn_shell::file_editor::{FileEditor, FileEditorAction};
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
        path: &str,
        _continuation: Option<u32>,
        output: &mut DirectoryPage,
    ) -> Result<(), Status> {
        if !self.directory_exists(path)? {
            return Err(Status::NOT_FOUND)
        }
        output.clear();
        Ok(())
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
        b"ok\ndirectory not empty\naccess denied\nok\naccess denied\naccess denied\nnot found\ndirectory not empty\n"
    );
}

struct RemoteEditorShell {
    command: Vec<u8>,
    output: Vec<u8>,
    file: Vec<u8>,
    version: u32,
    editor: Option<FileEditor<128>>,
    discard_prompt: bool,
    terminal: TerminalSize,
    resized: Option<TerminalSize>,
    closed: bool,
}

impl RemoteEditorShell {
    fn new(terminal: TerminalSize) -> Self {
        Self {
            command: Vec::new(),
            output: Vec::new(),
            file: Vec::new(),
            version: 1,
            editor: None,
            discard_prompt: false,
            terminal,
            resized: None,
            closed: false,
        }
    }

    fn render_editor(&mut self, editor: &mut FileEditor<128>) -> Result<(), SshError> {
        let rendered = editor
            .render::<8192>(self.terminal.columns as usize, self.terminal.rows as usize)
            .map_err(|_| SshError::WouldBlock)?;
        self.output.extend_from_slice(rendered.as_str().as_bytes());
        Ok(())
    }

    fn open_editor(&mut self) -> Result<(), SshError> {
        let mut editor = FileEditor::new("/note", self.version, &self.file)
            .map_err(|_| SshError::WouldBlock)?;
        self.render_editor(&mut editor)?;
        self.editor = Some(editor);
        Ok(())
    }

    fn save_editor(&mut self, editor: &mut FileEditor<128>) {
        self.file.clear();
        self.file.extend_from_slice(editor.bytes());
        self.version += 1;
        editor.mark_saved(self.version);
        self.output.extend_from_slice(
            format!(
                "EDIT operation=SAVED path=/note size={} version={}\n",
                editor.len(),
                editor.version(),
            )
            .as_bytes(),
        );
    }

    fn finish_editor(&mut self, operation: &str, editor: FileEditor<128>) {
        self.output.extend_from_slice(
            format!(
                "EDIT operation={operation} path=/note size={} version={}\n",
                editor.len(),
                editor.version(),
            )
            .as_bytes(),
        );
    }

    fn handle_editor_byte(&mut self, byte: u8) -> Result<(), SshError> {
        if self.discard_prompt {
            self.discard_prompt = false;
            if matches!(byte, b'y' | b'Y') {
                if let Some(editor) = self.editor.take() {
                    self.finish_editor("DISCARDED", editor);
                }
            } else if let Some(mut editor) = self.editor.take() {
                self.render_editor(&mut editor)?;
                self.editor = Some(editor);
            }
            return Ok(())
        }

        let key = match byte {
            19 => Key::Save,
            24 => Key::DiscardExit,
            26 => Key::SaveExit,
            _ => Key::Character(byte as char),
        };
        let mut editor = self.editor.take().ok_or(SshError::InvalidSession)?;
        let action = editor.handle(key).map_err(|_| SshError::WouldBlock)?;
        match action {
            FileEditorAction::Save | FileEditorAction::SaveExit => {
                let save_exit = action == FileEditorAction::SaveExit;
                if editor.is_dirty() {
                    self.save_editor(&mut editor);
                }
                if save_exit {
                    self.finish_editor("SAVED", editor);
                } else {
                    self.render_editor(&mut editor)?;
                    self.editor = Some(editor);
                }
            }
            FileEditorAction::PromptDiscard => {
                self.discard_prompt = true;
                self.output.extend_from_slice(b"Discard? [y/N]\n");
                self.editor = Some(editor);
            }
            FileEditorAction::DiscardExit => self.finish_editor("DISCARDED", editor),
            FileEditorAction::Redraw => {
                self.render_editor(&mut editor)?;
                self.editor = Some(editor);
            }
            FileEditorAction::None => self.editor = Some(editor),
        }
        Ok(())
    }

    fn handle_command(&mut self, command: &[u8]) -> Result<(), SshError> {
        match command {
            b"EDIT /note" | b"EDT /note" => self.open_editor(),
            b"TYPE /note" => {
                self.output.extend_from_slice(&self.file);
                self.output.push(b'\n');
                Ok(())
            }
            _ => Err(SshError::WouldBlock),
        }
    }
}

impl ShellBackend for RemoteEditorShell {
    type Handle = u64;

    fn open(
        &mut self,
        _principal: AuthenticatedPrincipal,
        terminal: TerminalSize,
    ) -> Result<Self::Handle, SshError> {
        self.terminal = terminal;
        Ok(1)
    }

    fn input(&mut self, _handle: Self::Handle, bytes: &[u8]) -> Result<usize, SshError> {
        for byte in bytes {
            if self.editor.is_some() {
                self.handle_editor_byte(*byte)?;
            } else {
                self.command.push(*byte);
                if *byte == b'\n' {
                    let command = self.command[..self.command.len() - 1].to_vec();
                    self.command.clear();
                    self.handle_command(&command)?;
                }
            }
        }
        Ok(bytes.len())
    }

    fn resize(&mut self, _handle: Self::Handle, terminal: TerminalSize) -> Result<(), SshError> {
        self.terminal = terminal;
        self.resized = Some(terminal);
        if let Some(mut editor) = self.editor.take() {
            self.render_editor(&mut editor)?;
            self.editor = Some(editor);
        }
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
fn remote_terminal_editor_saves_reopens_discards_and_resizes() {
    let terminal = TerminalSize::new(80, 24).unwrap();
    let mut daemon = SshDaemon::<_, _, 1>::new(Auth, RemoteEditorShell::new(terminal));
    let session = daemon
        .open_public_key_session("user", b"key", b"sig", b"hash", terminal)
        .expect("remote editor session opens");

    daemon.input(session, b"EDIT /note\nremote text").unwrap();
    daemon.resize(session, TerminalSize::new(100, 30).unwrap()).unwrap();
    daemon.input(session, &[19, 26]).unwrap();
    daemon.input(session, b"EDT /note\nremote text").unwrap();
    daemon.input(session, &[24, b'y']).unwrap();
    daemon.input(session, b"TYPE /note\n").unwrap();

    let mut output = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let count = daemon.output(session, &mut chunk).unwrap();
        if count == 0 {
            break
        }
        output.extend_from_slice(&chunk[..count]);
    }
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("EDIT operation=SAVED path=/note size=11 version=2"));
    assert!(output.contains("EDIT operation=DISCARDED path=/note size=22 version=2"));
    assert!(output.ends_with("remote text\n"));

    daemon.close(session).unwrap();
}
