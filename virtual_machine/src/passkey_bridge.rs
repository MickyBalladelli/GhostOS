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

const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <title>GhostOS Passkey</title>
  <link rel="icon" href="/icon.png" type="image/png">
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <main>
    <img class="mark" src="/icon.png" alt="GhostOS">
    <p class="eyebrow">GHOSTOS LOCAL SECURITY</p>
    <h1 id="title">Passkey</h1>
    <p id="message" class="message">Connecting to GhostOS…</p>
    <form id="form" hidden>
      <label for="username">Administrator username</label>
      <input id="username" name="username" autocomplete="username" maxlength="32" required>
      <button id="action" type="submit">Create passkey</button>
    </form>
    <p id="error" class="error" role="alert"></p>
    <p class="foot">Your private passkey stays in your authenticator.</p>
  </main>
  <script src="/app.js" defer></script>
</body>
</html>
"#;

const STYLE: &str = r#"*{box-sizing:border-box}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#07090c;color:#f4f7fb;font-family:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{width:min(92vw,440px);padding:42px;border:1px solid #252b35;border-radius:24px;background:linear-gradient(145deg,#12161d,#0b0e13);box-shadow:0 30px 80px #0009}.mark{display:block;width:72px;height:72px;object-fit:contain}.eyebrow{margin:28px 0 10px;color:#9ca6b5;font-size:12px;font-weight:700;letter-spacing:.16em}h1{margin:0;font-size:42px;letter-spacing:-.04em}.message{min-height:48px;margin:16px 0 28px;color:#bdc5d1;line-height:1.5}form{display:grid;gap:12px}form[hidden]{display:none}label{font-size:13px;font-weight:700;color:#d8dee8}input{width:100%;border:1px solid #303846;border-radius:12px;background:#080b10;color:#fff;padding:14px 15px;font:inherit;outline:none}input:focus{border-color:#e7ff57;box-shadow:0 0 0 3px #e7ff5722}button{margin-top:8px;border:0;border-radius:12px;background:#e7ff57;color:#080a0d;padding:15px;font:inherit;font-weight:850;cursor:pointer}button:disabled{cursor:wait;opacity:.55}.error{min-height:24px;color:#ff8585;font-size:14px}.foot{margin:26px 0 0;padding-top:20px;border-top:1px solid #252b35;color:#7f8998;font-size:12px}@media(max-width:520px){main{padding:28px;border-radius:18px}h1{font-size:36px}}"#;

const SCRIPT: &str = r#"const code = new URLSearchParams(location.search).get('code') || ''
const form = document.querySelector('#form')
const username = document.querySelector('#username')
const action = document.querySelector('#action')
const title = document.querySelector('#title')
const message = document.querySelector('#message')
const error = document.querySelector('#error')
let mode = ''
let busy = false
let pending = ''

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

function base64urlBytes(value) {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/')
    + '='.repeat((4 - value.length % 4) % 4)
  const decoded = atob(padded)
  return Uint8Array.from(decoded, byte => byte.charCodeAt(0))
}

function canonicalCoseKey(x, y) {
  if (!(x instanceof Uint8Array) || x.length !== 32
      || !(y instanceof Uint8Array) || y.length !== 32) {
    throw new Error('Authenticator did not return a usable ES256 public key')
  }
  return new Uint8Array([0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20, ...x, 0x22, 0x58, 0x20, ...y])
}

async function coseKey(response) {
  const publicKey = response.getPublicKey?.()
  if (publicKey) {
    const imported = await crypto.subtle.importKey(
      'spki',
      publicKey,
      {name: 'ECDSA', namedCurve: 'P-256'},
      true,
      ['verify']
    )
    const jwk = await crypto.subtle.exportKey('jwk', imported)
    if (jwk.kty !== 'EC' || jwk.crv !== 'P-256' || !jwk.x || !jwk.y) {
      throw new Error('Unsupported passkey public key')
    }
    return canonicalCoseKey(base64urlBytes(jwk.x), base64urlBytes(jwk.y))
  }

  const attestation = cborValue(new Uint8Array(response.attestationObject))
  const authenticator = attestation.value instanceof Map
    ? attestation.value.get('authData')
    : null
  if (!(authenticator instanceof Uint8Array) || authenticator.length < 55) {
    throw new Error('Authenticator did not return a usable ES256 public key')
  }
  const credentialLength = authenticator[53] * 256 + authenticator[54]
  const keyStart = 55 + credentialLength
  if (keyStart >= authenticator.length) throw new Error('Authenticator returned invalid credential data')
  const key = cborValue(authenticator, keyStart)
  if (!(key.value instanceof Map)
      || key.value.get(1) !== 2
      || key.value.get(3) !== -7
      || key.value.get(-1) !== 1) {
    throw new Error('Authenticator did not return an ES256 public key')
  }
  const x = key.value.get(-2)
  const y = key.value.get(-3)
  const canonical = canonicalCoseKey(x, y)
  const point = new Uint8Array(65)
  point[0] = 4
  point.set(x, 1)
  point.set(y, 33)
  await crypto.subtle.importKey(
    'raw',
    point,
    {name: 'ECDSA', namedCurve: 'P-256'},
    false,
    ['verify']
  )
  return canonical
}

function cborValue(bytes, start = 0) {
  if (start >= bytes.length) throw new Error('Truncated authenticator data')
  const first = bytes[start]
  const major = first >> 5
  const additional = first & 31
  let cursor = start + 1
  let length = additional
  if (additional === 24) {
    if (cursor >= bytes.length) throw new Error('Truncated authenticator data')
    length = bytes[cursor++]
  } else if (additional === 25) {
    if (cursor + 2 > bytes.length) throw new Error('Truncated authenticator data')
    length = bytes[cursor] * 256 + bytes[cursor + 1]
    cursor += 2
  } else if (additional === 26) {
    if (cursor + 4 > bytes.length) throw new Error('Truncated authenticator data')
    length = new DataView(bytes.buffer, bytes.byteOffset + cursor, 4).getUint32(0)
    cursor += 4
  } else if (additional >= 27) {
    throw new Error('Unsupported authenticator data')
  }

  if (major === 0) return {value: length, next: cursor}
  if (major === 1) return {value: -1 - length, next: cursor}
  if (major === 2 || major === 3) {
    const end = cursor + length
    if (end > bytes.length) throw new Error('Truncated authenticator data')
    const value = major === 2
      ? bytes.slice(cursor, end)
      : new TextDecoder().decode(bytes.slice(cursor, end))
    return {value, next: end}
  }
  if (major === 4) {
    const value = []
    for (let index = 0; index < length; index += 1) {
      const item = cborValue(bytes, cursor)
      value.push(item.value)
      cursor = item.next
    }
    return {value, next: cursor}
  }
  if (major === 5) {
    const value = new Map()
    for (let index = 0; index < length; index += 1) {
      const key = cborValue(bytes, cursor)
      const item = cborValue(bytes, key.next)
      value.set(key.value, item.value)
      cursor = item.next
    }
    return {value, next: cursor}
  }
  if (major === 7 && additional === 20) return {value: false, next: cursor}
  if (major === 7 && additional === 21) return {value: true, next: cursor}
  if (major === 7 && additional === 22) return {value: null, next: cursor}
  throw new Error('Unsupported authenticator data')
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

async function waitForEnrollment() {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const state = await api('/api/state')
    if (state.mode === 'enroll') return
    // Setup may already be finished (administrator exists and the bridge
    // flipped to login): that is success, not a timeout.
    if (state.mode === 'login' || state.mode === 'success') return
    if (state.error) throw new Error(state.error)
    await new Promise(resolve => setTimeout(resolve, 250))
  }
  throw new Error('GhostOS setup is not ready yet')
}

async function enroll(name) {
  await waitForEnrollment()
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
  const key = await coseKey(created.response)
  await api('/api/enroll', `${encodeURIComponent(name)}\n${bytesToHex(key)}`)
  localStorage.setItem('ghostos-username', name)
  pending = 'enroll'
  message.textContent = 'Passkey created. GhostOS is finishing setup and unlocking…'
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
  pending = 'login'
  message.textContent = 'Assertion sent. GhostOS is verifying it…'
}

function render(state) {
  mode = state.mode
  if (mode === 'waiting') form.hidden = true
  if (busy) return
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
    pending = ''
    title.textContent = 'Unlocked'
    message.textContent = 'Authentication accepted. You may close this page.'
    form.hidden = true
  } else if (pending === 'enroll') {
    title.textContent = 'Finishing setup'
    message.textContent = 'Passkey created. GhostOS is creating your administrator and unlocking…'
    form.hidden = true
  } else if (pending === 'login') {
    title.textContent = 'Unlocking'
    message.textContent = 'GhostOS is verifying your passkey…'
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
  form.hidden = true
  action.disabled = true
  error.textContent = ''
  try {
    if (!window.PublicKeyCredential) throw new Error('This browser does not support passkeys')
    if (mode === 'enroll') await enroll(name)
    else await login(name)
  } catch (problem) {
    pending = ''
    error.textContent = mode === 'enroll'
      ? `GhostOS setup did not finish: ${problem.message || String(problem)}`
      : (problem.message || String(problem))
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

        // Only the explicit commit line marks the administrator as
        // committed. The "terminal is locked" notice is NOT reliable:
        // on a first boot it can be printed before the enroll banner
        // appears, which would wrongly flip the bridge into login mode.
        // Locked reboots are handled below via the GhostOSLogin banner.
        if self.guest_text.contains("Administrator account committed.") {
            self.administrator_committed = true;
        }
        if self.administrator_committed
            && matches!(self.mode, Mode::Waiting | Mode::Enroll)
        {
            self.mode = Mode::Login;
            self.error = None;
            self.challenge = None;
            self.input_flow = InputFlow::None;
            self.login_in_progress = false;
        }
        let enrollment_pending = !self.administrator_committed
            && enrollment_pending(&self.guest_text);
        if enrollment_pending {
            self.mode = Mode::Enroll;
            self.challenge = None;
        }
        if new_text.contains("Administrator account committed.") {
            self.mode = Mode::Login;
            self.error = None;
            self.input_flow = InputFlow::None;
        }
        if self.login_in_progress && !enrollment_pending {
            if let Some(challenge) = last_challenge(&self.guest_text) {
                self.challenge = Some(challenge);
                self.mode = Mode::Challenge;
            }
        }
        if authentication_succeeded(&self.guest_text) {
            self.mode = Mode::Success;
            self.error = None;
            self.login_in_progress = false;
            self.input_flow = InputFlow::None;
        } else if new_text.contains("Login failed:") {
            self.mode = Mode::Login;
            self.challenge = None;
            self.error = Some("GhostOS rejected that passkey. Try again.".to_string());
            self.login_in_progress = false;
        } else if new_text.contains("Username must be 1-32 valid characters.")
            || new_text.contains("Unknown credential type.")
            || new_text.contains("Credential material is invalid.")
            || new_text.contains("Credential rejected.")
            || new_text.contains("Confirmation rejected.")
        {
            self.mode = Mode::Enroll;
            self.input_flow = InputFlow::None;
            self.error = Some("GhostOS could not save that passkey. Create it again.".to_string());
            self.login_in_progress = false;
        } else if !self.login_in_progress && login_pending(&self.guest_text) {
            self.mode = Mode::Login;
            self.challenge = None;
        }

        self.advance_input(vm);
    }

    fn advance_input(&mut self, vm: &mut Vm) {
        if matches!(self.mode, Mode::Success) {
            return
        }
        let next = match &self.input_flow {
            InputFlow::EnrollKind { key }
                if self
                    .guest_text
                    .contains("Credential type [PASSKEY/TPM/SSH] (PASSKEY):") =>
            {
                Some((bridge_line(b""), InputFlow::EnrollMaterial { key: key.clone() }))
            }
            InputFlow::EnrollMaterial { key }
                if self
                    .guest_text
                    .contains("Waiting for passkey public key from local browser:") =>
            {
                decode_hex(key)
                    .and_then(|material| bridge_binary_frame(&material, 96))
                    .map(|material| (material, InputFlow::EnrollConfirm))
            }
            InputFlow::EnrollConfirm
                if self
                    .guest_text
                    .contains("Create this administrator account? [y/N]:") =>
            {
                Some((bridge_line(b"y"), InputFlow::None))
            }
            InputFlow::LoginKind
                if self.guest_text.contains("\x1b]GhostOSAuthMethod\x07") =>
            {
                Some((bridge_line(b""), InputFlow::None))
            }
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

fn enrollment_pending(text: &str) -> bool {
    if text.contains("Administrator account committed.") {
        return false
    }
    // Compare banner positions: a stale enroll banner from an earlier boot
    // must not win over a fresher login banner on a locked start.
    let enroll = text.rfind("\x1b]GhostOSEnroll\x07");
    let login = text.rfind("\x1b]GhostOSLogin\x07");
    match (enroll, login) {
        (Some(enroll_at), Some(login_at)) => enroll_at > login_at,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn authentication_succeeded(text: &str) -> bool {
    let latest_marker = text
        .rfind("\x1b]GhostOSEnroll\x07")
        .max(text.rfind("\x1b]GhostOSLogin\x07"));
    if text
        .rfind("Login accepted.")
        .is_some_and(|accepted| latest_marker.is_none_or(|marker| accepted > marker))
    {
        return true
    }
    let Some(shell) = text.rfind("GhostOS user shell") else {
        return false
    };
    latest_marker.is_none_or(|marker| shell > marker) && text[shell..].contains("$ ")
}

fn login_pending(text: &str) -> bool {
    let Some(login) = text.rfind("\x1b]GhostOSLogin\x07") else {
        return false
    };
    let enroll = text.rfind("\x1b]GhostOSEnroll\x07");
    let accepted = text.rfind("Login accepted.");
    let shell = text.rfind("GhostOS user shell");
    enroll.is_none_or(|event| login > event)
        && accepted.is_none_or(|event| login > event)
        && shell.is_none_or(|event| login > event)
}

fn bridge_line(bytes: &[u8]) -> Vec<u8> {
    let mut line = Vec::with_capacity(bytes.len() + 1);
    line.extend_from_slice(bytes);
    line.push(b'\r');
    line
}

fn bridge_username_frame(username: &[u8]) -> Option<Vec<u8>> {
    if username.is_empty() || username.len() > 32 {
        return None
    }
    bridge_text_frame(username)
}

fn bridge_text_frame(text: &[u8]) -> Option<Vec<u8>> {
    let length = u16::try_from(text.len()).ok()?;
    if length == 0 {
        return None
    }
    let mut frame = Vec::with_capacity(text.len() + 3);
    frame.push(0);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(text);
    Some(frame)
}

fn bridge_assertion_frame(hex: &str) -> Option<Vec<u8>> {
    let assertion = decode_hex(hex)?;
    bridge_binary_frame(&assertion, 512)
}

fn bridge_binary_frame(bytes: &[u8], maximum: usize) -> Option<Vec<u8>> {
    let length = u16::try_from(bytes.len()).ok()?;
    if length == 0 || bytes.len() > maximum {
        return None
    }
    let mut frame = Vec::with_capacity(bytes.len() + 3);
    frame.push(0);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(bytes);
    Some(frame)
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
                client.last_activity = Instant::now();
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

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect()
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
    const CHALLENGE_MARKER: &str = "\x1b]GhostOSChallenge:";
    let start = text.rfind(CHALLENGE_MARKER)? + CHALLENGE_MARKER.len();
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
