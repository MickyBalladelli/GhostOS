use std::vec::Vec;

use synos_webterm::{
    AuthenticatedPrincipal, ShellBackend, SshAuthenticator, SshDaemon, SshError, Terminal,
    TerminalSize,
};

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
