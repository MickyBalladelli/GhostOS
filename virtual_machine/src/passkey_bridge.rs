use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

use ghostos_vm::Vm;

const MAX_CLIENTS: usize = 8;
const MAX_REQUEST_BYTES: usize = 16 * 1024;

const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <title>GhostOS Passkey</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <main>
    <div class="mark">G</div>
    <p class="eyebrow">GHOSTOS LOCAL SECURITY</p>
    <h1 id="title">Passkey</h1>
    <p id="message" class="message">Connecting to GhostOS…</p>
    <form id="form" hidden>
      <label for="username">Administrator username</label>
      <input id="username" name="username" autocomplete="username" maxlength="32" required>
      <button id="action" type="submit">Continue</button>
    </form>
    <p id="error" class="error" role="alert"></p>
    <p class="foot">Your private passkey stays in your authenticator.</p>
  </main>
  <script src="/app.js" defer></script>
</body>
</html>
"#;

const STYLE: &str = r#"*{box-sizing:border-box}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#07090c;color:#f4f7fb;font-family:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{width:min(92vw,440px);padding:42px;border:1px solid #252b35;border-radius:24px;background:linear-gradient(145deg,#12161d,#0b0e13);box-shadow:0 30px 80px #0009}.mark{display:grid;place-items:center;width:48px;height:48px;border-radius:14px;background:#e7ff57;color:#080a0d;font-size:25px;font-weight:900}.eyebrow{margin:28px 0 10px;color:#9ca6b5;font-size:12px;font-weight:700;letter-spacing:.16em}h1{margin:0;font-size:42px;letter-spacing:-.04em}.message{min-height:48px;margin:16px 0 28px;color:#bdc5d1;line-height:1.5}form{display:grid;gap:12px}label{font-size:13px;font-weight:700;color:#d8dee8}input{width:100%;border:1px solid #303846;border-radius:12px;background:#080b10;color:#fff;padding:14px 15px;font:inherit;outline:none}input:focus{border-color:#e7ff57;box-shadow:0 0 0 3px #e7ff5722}button{margin-top:8px;border:0;border-radius:12px;background:#e7ff57;color:#080a0d;padding:15px;font:inherit;font-weight:850;cursor:pointer}button:disabled{cursor:wait;opacity:.55}.error{min-height:24px;color:#ff8585;font-size:14px}.foot{margin:26px 0 0;padding-top:20px;border-top:1px solid #252b35;color:#7f8998;font-size:12px}@media(max-width:520px){main{padding:28px;border-radius:18px}h1{font-size:36px}}"#;

const SCRIPT: &str = r#"const code = new URLSearchParams(location.search).get('code') || ''
const form = document.querySelector('#form')
const username = document.querySelector('#username')
const action = document.querySelector('#action')
const title = document.querySelector('#title')
const message = document.querySelector('#message')
const error = document.querySelector('#error')
let mode = ''
let busy = false

username.value = localStorage.getItem('ghostos-username') || ''

function bytesToHex(bytes) {
  return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('')
}

function hexToBytes(hex) {
  const bytes = new Uint8Array(hex.length / 2)
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16)
  }
  return bytes
}

async function api(path, body) {
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: {'X-GhostOS-Code': code, 'Content-Type': 'text/plain'},
    body
  })
  const text = await response.text()
  if (!response.ok) throw new Error(text || 'GhostOS request failed')
  return text ? JSON.parse(text) : {}
}

function coseKey(response) {
  const publicKey = response.getPublicKey?.()
  if (!publicKey) throw new Error('This authenticator did not return an ES256 public key')
  const spki = new Uint8Array(publicKey)
  let point = -1
  for (let index = spki.length - 65; index >= 0; index -= 1) {
    if (spki[index] === 4 && spki.length - index >= 65) {
      point = index
      break
    }
  }
  if (point < 0 || spki.length - point !== 65) throw new Error('Unsupported passkey public key')
  const x = spki.slice(point + 1, point + 33)
  const y = spki.slice(point + 33, point + 65)
  return new Uint8Array([0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20, ...x, 0x22, 0x58, 0x20, ...y])
}

function packedAssertion(response) {
  const authenticator = new Uint8Array(response.authenticatorData)
  const client = new Uint8Array(response.clientDataJSON)
  const signature = new Uint8Array(response.signature)
  const packed = new Uint8Array(12 + authenticator.length + client.length + signature.length)
  packed.set([0x53, 0x59, 0x57, 0x42, 1, 0, 0, 0])
  packed[8] = authenticator.length & 255
  packed[9] = authenticator.length >> 8
  packed[10] = client.length & 255
  packed[11] = client.length >> 8
  packed.set(authenticator, 12)
  packed.set(client, 12 + authenticator.length)
  packed.set(signature, 12 + authenticator.length + client.length)
  return packed
}

async function waitForChallenge() {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const state = await api('/api/state')
    if (state.challenge) return state.challenge
    if (state.error) throw new Error(state.error)
    await new Promise(resolve => setTimeout(resolve, 250))
  }
  throw new Error('GhostOS did not provide a login challenge')
}

async function enroll(name) {
  const created = await navigator.credentials.create({publicKey: {
    challenge: crypto.getRandomValues(new Uint8Array(32)),
    rp: {id: 'localhost', name: 'GhostOS'},
    user: {
      id: crypto.getRandomValues(new Uint8Array(32)),
      name,
      displayName: name
    },
    pubKeyCredParams: [{type: 'public-key', alg: -7}],
    authenticatorSelection: {residentKey: 'required', userVerification: 'required'},
    attestation: 'none',
    timeout: 120000
  }})
  const key = coseKey(created.response)
  await api('/api/enroll', `${encodeURIComponent(name)}\n${bytesToHex(key)}`)
  localStorage.setItem('ghostos-username', name)
  message.textContent = 'Passkey created. GhostOS is creating your administrator account…'
}

async function login(name) {
  await api('/api/login/start', encodeURIComponent(name))
  message.textContent = 'Touch your passkey to sign in.'
  const challenge = await waitForChallenge()
  const credential = await navigator.credentials.get({publicKey: {
    challenge: hexToBytes(challenge),
    rpId: 'localhost',
    userVerification: 'required',
    timeout: 120000
  }})
  await api('/api/login/complete', bytesToHex(packedAssertion(credential.response)))
  localStorage.setItem('ghostos-username', name)
  message.textContent = 'Assertion sent. GhostOS is verifying it…'
}

function render(state) {
  if (busy) return
  mode = state.mode
  error.textContent = state.error || ''
  if (mode === 'enroll') {
    title.textContent = 'Create administrator'
    message.textContent = 'Create a passkey for this GhostOS machine.'
    action.textContent = 'Create passkey'
    form.hidden = false
  } else if (mode === 'login') {
    title.textContent = 'Unlock GhostOS'
    message.textContent = 'Sign in with your passkey.'
    action.textContent = 'Use passkey'
    form.hidden = false
  } else if (mode === 'success') {
    title.textContent = 'Unlocked'
    message.textContent = 'Authentication accepted. You may close this page.'
    form.hidden = true
  } else {
    title.textContent = 'Waiting for GhostOS'
    message.textContent = 'Keep this page open while the machine starts.'
    form.hidden = true
  }
}

form.addEventListener('submit', async event => {
  event.preventDefault()
  if (busy) return
  const name = username.value.trim()
  if (!/^[A-Za-z0-9._$-]{1,32}$/.test(name)) {
    error.textContent = 'Use 1–32 letters, numbers, or . _ - $'
    return
  }
  busy = true
  action.disabled = true
  error.textContent = ''
  try {
    if (!window.PublicKeyCredential) throw new Error('This browser does not support passkeys')
    if (mode === 'enroll') await enroll(name)
    else await login(name)
  } catch (problem) {
    error.textContent = problem.message || String(problem)
  } finally {
    busy = false
    action.disabled = false
  }
})

async function refresh() {
  try {
    render(await api('/api/state'))
  } catch (problem) {
    error.textContent = problem.message || String(problem)
  }
}

refresh()
setInterval(refresh, 750)
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Waiting,
    Enroll,
    Login,
    Challenge,
    Success,
}

struct Client {
    stream: TcpStream,
    bytes: Vec<u8>,
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
    observed_output: usize,
    guest_text: String,
    mode: Mode,
    challenge: Option<String>,
    error: Option<String>,
}

impl PasskeyBridge {
    pub fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let url = format!("http://localhost:{port}/?code={token}");
        Ok(Self {
            listener,
            clients: Vec::new(),
            url,
            token,
            observed_output: 0,
            guest_text: String::new(),
            mode: Mode::Waiting,
            challenge: None,
            error: None,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn poll(&mut self, vm: &mut Vm) -> io::Result<()> {
        if let Some(serial) = vm.serial() {
            serial.borrow_mut().set_authentication_banner(
                format!("Passkey setup and login: {}\r\n", self.url).as_bytes(),
            );
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
            let (status, content_type, body) = self.handle(request, vm);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'self'; connect-src 'self'; script-src 'self'; style-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.set_nonblocking(false)?;
            stream.write_all(response.as_bytes())?;
            stream.write_all(body.as_bytes())?;
        }
        Ok(())
    }

    fn observe_guest(&mut self, vm: &Vm) {
        let Some(serial) = vm.serial() else { return };
        let serial = serial.borrow();
        let output = serial.output();
        if output.len() < self.observed_output {
            self.observed_output = 0;
            self.guest_text.clear();
        }
        if output.len() == self.observed_output {
            return
        }
        self.guest_text
            .push_str(&String::from_utf8_lossy(&output[self.observed_output..]));
        self.observed_output = output.len();
        if self.guest_text.len() > 64 * 1024 {
            let keep = self.guest_text.len() - 32 * 1024;
            self.guest_text.drain(..keep);
        }

        if self.guest_text.contains("Administrator username: ")
            && !self.guest_text.contains("Administrator account committed.")
        {
            self.mode = Mode::Enroll;
        }
        if self.guest_text.contains("Administrator account committed.") {
            self.mode = Mode::Login;
        }
        if let Some(challenge) = last_challenge(&self.guest_text) {
            self.challenge = Some(challenge);
            self.mode = Mode::Challenge;
        }
        if self.guest_text.contains("Login accepted.") {
            self.mode = Mode::Success;
            self.error = None;
        } else if self.guest_text.contains("Login failed:") {
            self.mode = Mode::Login;
            self.challenge = None;
            self.error = Some("GhostOS rejected that passkey. Try again.".to_string());
        } else if self.mode == Mode::Waiting
            && self.guest_text.contains("Username: ")
            && !self.guest_text.contains("Administrator username: ")
        {
            self.mode = Mode::Login;
        }
    }

    fn accept_clients(&mut self) -> io::Result<()> {
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
            return ("200 OK", "text/html; charset=utf-8", PAGE.to_string())
        }
        if request.method == "GET" && path == "/style.css" {
            return ("200 OK", "text/css; charset=utf-8", STYLE.to_string())
        }
        if request.method == "GET" && path == "/app.js" {
            return ("200 OK", "text/javascript; charset=utf-8", SCRIPT.to_string())
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
            if self.mode != Mode::Enroll {
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
            if !valid_username(&username) || !valid_hex(key, 192) {
                return text_response("400 Bad Request", "Invalid username or passkey")
            }
            self.guest_text.clear();
            self.observed_output = vm.serial().map_or(0, |serial| serial.borrow().output().len());
            vm.queue_serial_input(format!("{username}\rpasskey\r{key}\ry\r").as_bytes());
            self.mode = Mode::Waiting;
            self.error = None;
            return json_ok()
        }
        if request.method == "POST" && path == "/api/login/start" {
            if !matches!(self.mode, Mode::Login) {
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
            self.challenge = None;
            self.error = None;
            self.mode = Mode::Waiting;
            self.guest_text.clear();
            self.observed_output = vm.serial().map_or(0, |serial| serial.borrow().output().len());
            vm.queue_serial_input(format!("{username}\rpasskey\r").as_bytes());
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
            vm.queue_serial_input(format!("{assertion}\r").as_bytes());
            return json_ok()
        }
        text_response("404 Not Found", "Not found")
    }
}

fn read_request(client: &mut Client) -> io::Result<Option<Request>> {
    let mut buffer = [0; 4096];
    loop {
        match client.stream.read(&mut buffer) {
            Ok(0) if client.bytes.is_empty() => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(0) => break,
            Ok(count) => {
                if client.bytes.len() + count > MAX_REQUEST_BYTES {
                    return Err(io::ErrorKind::InvalidData.into())
                }
                client.bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error),
        }
    }
    let Some(header_end) = find_bytes(&client.bytes, b"\r\n\r\n") else {
        return Ok(None)
    };
    let headers = std::str::from_utf8(&client.bytes[..header_end])
        .map_err(|_| io::ErrorKind::InvalidData)?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    if body_start + content_length > client.bytes.len() {
        return Ok(None)
    }
    let mut lines = headers.lines();
    let first = lines.next().ok_or(io::ErrorKind::InvalidData)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().ok_or(io::ErrorKind::InvalidData)?.to_string();
    let target = parts.next().ok_or(io::ErrorKind::InvalidData)?.to_string();
    let token = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("x-ghostos-code")
            .then(|| value.trim().to_string())
    });
    Ok(Some(Request {
        method,
        target,
        token,
        body: client.bytes[body_start..body_start + content_length].to_vec(),
    }))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn query_value<'a>(target: &'a str, wanted: &str) -> Option<&'a str> {
    target.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == wanted).then_some(value)
    })
}

fn percent_decode(input: &str) -> Option<String> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                output.push((hex_digit(bytes[index + 1])? << 4) | hex_digit(bytes[index + 2])?);
                index += 3;
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte if byte.is_ascii() => {
                output.push(byte);
                index += 1;
            }
            _ => return None,
        }
    }
    String::from_utf8(output).ok()
}

fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 32
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-$".contains(&byte))
}

fn valid_hex(input: &str, maximum: usize) -> bool {
    !input.is_empty()
        && input.len() <= maximum
        && input.len() % 2 == 0
        && input.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn last_challenge(text: &str) -> Option<String> {
    let start = text.rfind("Challenge: ")? + "Challenge: ".len();
    let challenge = text.get(start..start + 64)?;
    challenge.bytes().all(|byte| byte.is_ascii_hexdigit()).then(|| challenge.to_string())
}

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
