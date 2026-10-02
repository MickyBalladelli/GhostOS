#[path = "passkey_native.rs"]
mod native;

use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use ghostos_vm::Vm;

const MAX_CLIENTS: usize = 8;
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const ICON: &[u8] = include_bytes!("../../icon.png");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Waiting,
    Enroll,
    Login,
    Challenge,
    Success,
}

enum InputFlow {
    None,
    EnrollKind { key: String },
    EnrollMaterial { key: String },
    EnrollConfirm,
    LoginKind,
}

struct Client {
    stream: TcpStream,
    bytes: Vec<u8>,
    last_activity: Instant,
}

struct Request {
    method: String,
    target: String,
    token: Option<String>,
    body: Vec<u8>,
}

pub struct PasskeyBridge {
    listener: TcpListener,
    clients: Vec<Client>,
    url: String,
    token: String,
    authentication_banner: Vec<u8>,
    observed_output: usize,
    guest_text: String,
    mode: Mode,
    challenge: Option<String>,
    error: Option<String>,
    input_flow: InputFlow,
    administrator_committed: bool,
    login_in_progress: bool,
}

impl PasskeyBridge {
    pub fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let url = format!("http://localhost:{port}/?code={token}");
        let authentication_banner = format!(
            "GhostOS authentication is open in your browser.\r\nIf it did not open: {url}\r\nDo not type credentials in this terminal.\r\n"
        )
        .into_bytes();
        Ok(Self {
            listener,
            clients: Vec::new(),
            url,
            token,
            authentication_banner,
            observed_output: 0,
            guest_text: String::new(),
            mode: Mode::Waiting,
            challenge: None,
            error: None,
            input_flow: InputFlow::None,
            administrator_committed: false,
            login_in_progress: false,
        })
    }

    pub fn open_in_browser(&self) {
        #[cfg(target_os = "macos")]
        let _ = Command::new("open").arg(&self.url).spawn();

        #[cfg(target_os = "linux")]
        let _ = Command::new("xdg-open").arg(&self.url).spawn();
    }

    pub fn poll(&mut self, vm: &mut Vm) -> io::Result<()> {
        if let Some(serial) = vm.serial() {
            serial
                .borrow_mut()
                .set_authentication_banner(&self.authentication_banner);
        }
        self.observe_guest(vm);
        self.accept_clients()?;
        let mut ready = Vec::new();
        for index in (0..self.clients.len()).rev() {
            match read_request(&mut self.clients[index]) {
                Ok(Some(request)) => {
                    let client = self.clients.swap_remove(index);
                    ready.push((client.stream, request));
                }
                Ok(None) => {}
                Err(_) => {
                    self.clients.swap_remove(index);
                }
            }
        }
        for (mut stream, request) in ready {
            let icon_request = request.method == "GET"
                && request.target.split('?').next() == Some("/icon.png");
            let (status, content_type, body) = if icon_request {
                ("200 OK", "image/png", Cow::Borrowed(ICON))
            } else {
                let (status, content_type, body) = self.handle(request, vm);
                (status, content_type, Cow::Owned(body.into_bytes()))
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'self'; connect-src 'self'; script-src 'self'; style-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if stream.set_nonblocking(false).is_err() {
                continue
            }
            let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));
            if stream.write_all(response.as_bytes()).is_ok() {
                let _ = stream.write_all(body.as_ref());
            }
        }
        Ok(())
    }

    fn observe_guest(&mut self, vm: &mut Vm) {
        let Some(serial) = vm.serial() else { return };
        let new_output = {
            let serial = serial.borrow();
            let output = serial.output();
            if output.len() < self.observed_output {
                self.observed_output = 0;
                self.guest_text.clear();
            }
            if output.len() == self.observed_output {
                return
            }
            let new_output = output[self.observed_output..].to_vec();
            self.observed_output = output.len();
            new_output
        };
        let new_text = String::from_utf8_lossy(&new_output);
        self.guest_text
            .push_str(&new_text);
        if self.guest_text.len() > 64 * 1024 {
            let keep = self.guest_text.len() - 32 * 1024;
            self.guest_text.drain(..keep);
        }

        native::observe(self, &new_text);

        self.advance_input(vm);
    }

    fn advance_input(&mut self, vm: &mut Vm) {
        let action = native::next_input(self.mode, &self.input_flow, &self.guest_text);
        let next = match (&self.input_flow, action) {
            (InputFlow::EnrollKind { key }, 1) =>
                Some((bridge_line(b""), InputFlow::EnrollMaterial { key: key.clone() })),
            (InputFlow::EnrollMaterial { key }, 2) => decode_hex(key)
                .and_then(|material| bridge_binary_frame(&material, 96))
                .map(|material| (material, InputFlow::EnrollConfirm)),
            (InputFlow::EnrollConfirm, 3) => Some((bridge_line(b"y"), InputFlow::None)),
            (InputFlow::LoginKind, 4) => Some((bridge_line(b""), InputFlow::None)),
            _ => None,
        };
        let Some((bytes, next_flow)) = next else { return };
        vm.queue_serial_input(&bytes);
        self.input_flow = next_flow;
    }

    fn accept_clients(&mut self) -> io::Result<()> {
        self.clients
            .retain(|client| client.last_activity.elapsed() < CLIENT_IDLE_TIMEOUT);
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() >= MAX_CLIENTS {
                        continue
                    }
                    stream.set_nonblocking(true)?;
                    self.clients.push(Client {
                        stream,
                        bytes: Vec::new(),
                        last_activity: Instant::now(),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    fn handle(&mut self, request: Request, vm: &mut Vm) -> (&'static str, &'static str, String) {
        let path = request.target.split('?').next().unwrap_or(&request.target);
        if request.method == "GET" && path == "/" {
            if query_value(&request.target, "code") != Some(self.token.as_str()) {
                return text_response("404 Not Found", "Not found")
            }
            return ("200 OK", "text/html; charset=utf-8", native::asset(0).to_string())
        }
        if request.method == "GET" && path == "/style.css" {
            return ("200 OK", "text/css; charset=utf-8", native::asset(1).to_string())
        }
        if request.method == "GET" && path == "/app.js" {
            return ("200 OK", "text/javascript; charset=utf-8", native::asset(2).to_string())
        }
        if !path.starts_with("/api/")
            || request.token.as_deref() != Some(self.token.as_str())
        {
            return text_response("403 Forbidden", "Invalid setup code")
        }
        if request.method == "GET" && path == "/api/state" {
            let mode = match self.mode {
                Mode::Waiting => "waiting",
                Mode::Enroll => "enroll",
                Mode::Login | Mode::Challenge => "login",
                Mode::Success => "success",
            };
            let mode = if mode == "waiting"
                && !self.administrator_committed
                && enrollment_pending(&self.guest_text)
            {
                "enroll"
            } else {
                mode
            };
            let challenge = self.challenge.as_deref().unwrap_or("");
            let error = self.error.as_deref().unwrap_or("");
            return (
                "200 OK",
                "application/json; charset=utf-8",
                format!(
                    "{{\"mode\":\"{mode}\",\"challenge\":\"{challenge}\",\"error\":\"{error}\"}}"
                ),
            )
        }
        if request.method == "POST" && path == "/api/enroll" {
            let first_run_waiting = !self.administrator_committed
                && enrollment_pending(&self.guest_text);
            if self.mode != Mode::Enroll && !first_run_waiting {
                return text_response("409 Conflict", "GhostOS is not waiting for enrollment")
            }
            let Ok(body) = std::str::from_utf8(&request.body) else {
                return text_response("400 Bad Request", "Invalid enrollment")
            };
            let Some((encoded_username, key)) = body.split_once('\n') else {
                return text_response("400 Bad Request", "Invalid enrollment")
            };
            let Some(username) = percent_decode(encoded_username) else {
                return text_response("400 Bad Request", "Invalid username")
            };
            if !valid_username(&username) || key.len() != 154 || !valid_hex(key, 154) {
                return text_response("400 Bad Request", "Invalid username or passkey")
            }
            self.guest_text.clear();
            self.observed_output = vm.serial().map_or(0, |serial| serial.borrow().output().len());
            let Some(username_frame) = bridge_username_frame(username.as_bytes()) else {
                return text_response("400 Bad Request", "Invalid username")
            };
            vm.queue_serial_input(&username_frame);
            self.input_flow = InputFlow::EnrollKind {
                key: key.to_string(),
            };
            self.mode = Mode::Waiting;
            self.error = None;
            self.login_in_progress = false;
            return json_ok()
        }
        if request.method == "POST" && path == "/api/login/start" {
            if !matches!(self.mode, Mode::Login | Mode::Challenge) {
                return text_response("409 Conflict", "GhostOS is not waiting for login")
            }
            let Ok(encoded_username) = std::str::from_utf8(&request.body) else {
                return text_response("400 Bad Request", "Invalid username")
            };
            let Some(username) = percent_decode(encoded_username) else {
                return text_response("400 Bad Request", "Invalid username")
            };
            if !valid_username(&username) {
                return text_response("400 Bad Request", "Invalid username")
            }
            if self.mode == Mode::Challenge {
                return json_ok()
            }
            self.challenge = None;
            self.error = None;
            self.mode = Mode::Waiting;
            self.guest_text.clear();
            self.observed_output = vm.serial().map_or(0, |serial| serial.borrow().output().len());
            let Some(username_frame) = bridge_username_frame(username.as_bytes()) else {
                return text_response("400 Bad Request", "Invalid username")
            };
            vm.queue_serial_input(&username_frame);
            self.input_flow = InputFlow::LoginKind;
            self.login_in_progress = true;
            return json_ok()
        }
        if request.method == "POST" && path == "/api/login/complete" {
            let Ok(assertion) = std::str::from_utf8(&request.body) else {
                return text_response("400 Bad Request", "Invalid assertion")
            };
            if self.mode != Mode::Challenge
                || !valid_hex(assertion, 1024)
                || !assertion.starts_with("53595742")
            {
                return text_response("400 Bad Request", "Invalid WebAuthn assertion")
            }
            self.error = None;
            self.mode = Mode::Waiting;
            self.guest_text.clear();
            self.observed_output = vm.serial().map_or(0, |serial| serial.borrow().output().len());
            let Some(assertion_frame) = bridge_assertion_frame(assertion) else {
                return text_response("400 Bad Request", "Invalid WebAuthn assertion")
            };
            vm.queue_serial_input(&assertion_frame);
            self.login_in_progress = false;
            return json_ok()
        }
        text_response("404 Not Found", "Not found")
    }
}

fn enrollment_pending(text: &str) -> bool { native::enrollment_pending(text) }

fn bridge_line(bytes: &[u8]) -> Vec<u8> {
    native::frame(bytes, 0, 0).expect("line frame always fits")
}

fn bridge_username_frame(username: &[u8]) -> Option<Vec<u8>> { native::frame(username, 1, 32) }

fn bridge_assertion_frame(hex: &str) -> Option<Vec<u8>> {
    bridge_binary_frame(&decode_hex(hex)?, 512)
}

fn bridge_binary_frame(bytes: &[u8], maximum: usize) -> Option<Vec<u8>> { native::frame(bytes, 2, maximum) }

fn read_request(client: &mut Client) -> io::Result<Option<Request>> {
    let mut buffer = [0; 4096];
    loop {
        match client.stream.read(&mut buffer) {
            Ok(0) if client.bytes.is_empty() => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(0) => break,
            Ok(count) => {
                if !native::request_fits(client.bytes.len(), count) {
                    return Err(io::ErrorKind::InvalidData.into())
                }
                client.bytes.extend_from_slice(&buffer[..count]);
                client.last_activity = Instant::now();
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error),
        }
    }
    native::parse_request(&client.bytes)
}

fn query_value<'a>(target: &'a str, wanted: &str) -> Option<&'a str> { native::query(target, wanted) }
fn percent_decode(input: &str) -> Option<String> { native::percent_decode(input) }
fn valid_username(username: &str) -> bool { native::valid_username(username) }
fn valid_hex(input: &str, maximum: usize) -> bool { native::valid_hex(input, maximum) }
fn decode_hex(input: &str) -> Option<Vec<u8>> { native::decode_hex(input) }

fn random_token() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut first = RandomState::new().build_hasher();
    first.write_u128(seed);
    first.write_u32(std::process::id());
    let mut second = RandomState::new().build_hasher();
    second.write_u128(seed.rotate_left(47));
    second.write_u32(std::process::id());
    format!("{:016x}{:016x}", first.finish(), second.finish())
}

fn text_response(status: &'static str, body: &str) -> (&'static str, &'static str, String) {
    (status, "text/plain; charset=utf-8", body.to_string())
}

fn json_ok() -> (&'static str, &'static str, String) {
    ("200 OK", "application/json; charset=utf-8", "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::PasskeyBridge;

    #[test]
    fn bind_listens_on_localhost_without_opening_a_browser() {
        let bridge = PasskeyBridge::bind().expect("bind local passkey bridge");
        assert!(bridge.url.contains("http://localhost:"));
        assert!(!bridge.authentication_banner.is_empty());
    }
}
