use core::fmt;

use synos_init::RestartPolicy;
use synos_status::{IntoStatus, Status};

pub const APP_MANIFEST_SCHEMA: u16 = 1;
pub const MAX_APP_NAME_BYTES: usize = 48;
pub const MAX_RESOURCE_NAME_BYTES: usize = 64;
pub const MAX_APP_CAPABILITIES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Capacity,
    DuplicateKey,
    InvalidBoolean,
    InvalidInteger,
    InvalidManifest,
    InvalidRestartPolicy,
    InvalidRights,
    InvalidString,
    MissingField,
    UnsupportedSchema,
    UnknownKey,
    UnknownSection,
    UnknownValue,
}

impl IntoStatus for ManifestError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::UnsupportedSchema | Self::UnknownKey | Self::UnknownSection => Status::NOT_FOUND,
            Self::DuplicateKey
            | Self::InvalidBoolean
            | Self::InvalidInteger
            | Self::InvalidManifest
            | Self::InvalidRestartPolicy
            | Self::InvalidRights
            | Self::InvalidString
            | Self::MissingField
            | Self::UnknownValue => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BoundedText<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    length: u8,
}

impl<const CAPACITY: usize> BoundedText<CAPACITY> {
    pub const EMPTY: Self = Self {
        bytes: [0; CAPACITY],
        length: 0,
    };

    pub fn new(value: &str) -> Result<Self, ManifestError> {
        if value.is_empty()
            || value.len() > CAPACITY
            || CAPACITY > u8::MAX as usize
            || value.as_bytes().contains(&0)
        {
            return Err(ManifestError::InvalidString);
        }
        let mut bytes = [0; CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize])
            .expect("BoundedText contains source UTF-8")
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
}

impl<const CAPACITY: usize> fmt::Debug for BoundedText<CAPACITY> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BoundedText")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationKind {
    Service,
    Interactive,
    Batch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    Local,
    AnyNode,
    Node(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartMode {
    Never,
    OnFailure,
    Always,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeSpec {
    pub placement: Placement,
    pub restart_mode: RestartMode,
    pub restart: RestartPolicy,
}

impl RuntimeSpec {
    const DEFAULT: Self = Self {
        placement: Placement::Local,
        restart_mode: RestartMode::Never,
        restart: RestartPolicy::NEVER,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CapabilityRights(u16);

impl CapabilityRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const CREATE: Self = Self(1 << 4);
    pub const SEND: Self = Self(1 << 5);
    pub const RECEIVE: Self = Self(1 << 6);
    pub const DELEGATE: Self = Self(1 << 7);
    pub const ALL: Self = Self((1 << 8) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    Ipc,
    SharedMemory,
    File,
    Network,
    Device,
    Actor,
    Clock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRequest {
    pub resource: BoundedText<MAX_RESOURCE_NAME_BYTES>,
    pub kind: CapabilityKind,
    pub rights: CapabilityRights,
    pub required: bool,
}

impl CapabilityRequest {
    pub(crate) const EMPTY: Self = Self {
        resource: BoundedText::EMPTY,
        kind: CapabilityKind::Ipc,
        rights: CapabilityRights::NONE,
        required: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppManifest {
    pub(crate) schema: u16,
    pub(crate) name: BoundedText<MAX_APP_NAME_BYTES>,
    pub(crate) image: u128,
    pub(crate) kind: ApplicationKind,
    pub(crate) runtime: RuntimeSpec,
    pub(crate) capabilities: [CapabilityRequest; MAX_APP_CAPABILITIES],
    pub(crate) capability_count: u8,
}

impl AppManifest {
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        Parser::new().parse(source)
    }

    pub fn capabilities(&self) -> impl Iterator<Item = CapabilityRequest> + '_ {
        self.capabilities[..self.capability_count as usize]
            .iter()
            .copied()
    }

    pub const fn schema(&self) -> u16 {
        self.schema
    }

    pub const fn name(&self) -> &BoundedText<MAX_APP_NAME_BYTES> {
        &self.name
    }

    pub const fn image(&self) -> u128 {
        self.image
    }

    pub const fn kind(&self) -> ApplicationKind {
        self.kind
    }

    pub const fn runtime(&self) -> RuntimeSpec {
        self.runtime
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    Root,
    Application,
    Runtime,
    Capability(usize),
}

struct Parser {
    manifest: AppManifest,
    section: Section,
    seen_root: u8,
    seen_application: u8,
    seen_runtime: u8,
    capability_seen: [u8; MAX_APP_CAPABILITIES],
}

impl Parser {
    fn new() -> Self {
        Self {
            manifest: AppManifest {
                schema: 0,
                name: BoundedText::EMPTY,
                image: 0,
                kind: ApplicationKind::Service,
                runtime: RuntimeSpec::DEFAULT,
                capabilities: [CapabilityRequest::EMPTY; MAX_APP_CAPABILITIES],
                capability_count: 0,
            },
            section: Section::Root,
            seen_root: 0,
            seen_application: 0,
            seen_runtime: 0,
            capability_seen: [0; MAX_APP_CAPABILITIES],
        }
    }

    fn parse(mut self, source: &str) -> Result<AppManifest, ManifestError> {
        for raw_line in source.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                self.enter_section(line)?
            } else {
                let (key, value) = line.split_once('=').ok_or(ManifestError::InvalidManifest)?;
                self.assign(key.trim(), value.trim())?
            }
        }
        self.finish()
    }

    fn enter_section(&mut self, line: &str) -> Result<(), ManifestError> {
        self.section = match line {
            "[application]" => Section::Application,
            "[runtime]" => Section::Runtime,
            "[[capability]]" => {
                let index = self.manifest.capability_count as usize;
                if index == MAX_APP_CAPABILITIES {
                    return Err(ManifestError::Capacity);
                }
                self.manifest.capability_count += 1;
                Section::Capability(index)
            }
            _ if line.starts_with("[[") => return Err(ManifestError::UnknownSection),
            _ => return Err(ManifestError::UnknownSection),
        };
        Ok(())
    }

    fn assign(&mut self, key: &str, value: &str) -> Result<(), ManifestError> {
        match self.section {
            Section::Root => match key {
                "schema" => {
                    mark(&mut self.seen_root, 1)?;
                    self.manifest.schema = parse_u64(value)?
                        .try_into()
                        .map_err(|_| ManifestError::InvalidInteger)?
                }
                _ => return Err(ManifestError::UnknownKey),
            },
            Section::Application => match key {
                "name" => {
                    mark(&mut self.seen_application, 1)?;
                    self.manifest.name = BoundedText::new(parse_string(value)?)?
                }
                "image" => {
                    mark(&mut self.seen_application, 2)?;
                    self.manifest.image = parse_image(parse_string(value)?)?
                }
                "kind" => {
                    mark(&mut self.seen_application, 4)?;
                    self.manifest.kind = match parse_string(value)? {
                        "service" => ApplicationKind::Service,
                        "interactive" => ApplicationKind::Interactive,
                        "batch" => ApplicationKind::Batch,
                        _ => return Err(ManifestError::UnknownValue),
                    }
                }
                _ => return Err(ManifestError::UnknownKey),
            },
            Section::Runtime => self.assign_runtime(key, value)?,
            Section::Capability(index) => self.assign_capability(index, key, value)?,
        }
        Ok(())
    }

    fn assign_runtime(&mut self, key: &str, value: &str) -> Result<(), ManifestError> {
        match key {
            "placement" => {
                mark(&mut self.seen_runtime, 1)?;
                let placement = parse_string(value)?;
                self.manifest.runtime.placement = match placement {
                    "local" => Placement::Local,
                    "any" => Placement::AnyNode,
                    _ => {
                        let raw = placement
                            .strip_prefix("node:")
                            .ok_or(ManifestError::UnknownValue)?;
                        let node = raw
                            .parse::<u32>()
                            .map_err(|_| ManifestError::InvalidInteger)?;
                        if node == 0 {
                            return Err(ManifestError::InvalidInteger);
                        }
                        Placement::Node(node)
                    }
                }
            }
            "restart" => {
                mark(&mut self.seen_runtime, 2)?;
                self.manifest.runtime.restart_mode = match parse_string(value)? {
                    "never" => RestartMode::Never,
                    "on-failure" => RestartMode::OnFailure,
                    "always" => RestartMode::Always,
                    _ => return Err(ManifestError::UnknownValue),
                }
            }
            "max_restarts" => {
                mark(&mut self.seen_runtime, 4)?;
                self.manifest.runtime.restart.max_restarts = parse_u16(value)?
            }
            "window_us" => {
                mark(&mut self.seen_runtime, 8)?;
                self.manifest.runtime.restart.window_us = parse_u64(value)?
            }
            "initial_backoff_us" => {
                mark(&mut self.seen_runtime, 16)?;
                self.manifest.runtime.restart.initial_backoff_us = parse_u64(value)?
            }
            "max_backoff_us" => {
                mark(&mut self.seen_runtime, 32)?;
                self.manifest.runtime.restart.max_backoff_us = parse_u64(value)?
            }
            _ => return Err(ManifestError::UnknownKey),
        }
        Ok(())
    }

    fn assign_capability(
        &mut self,
        index: usize,
        key: &str,
        value: &str,
    ) -> Result<(), ManifestError> {
        let seen = &mut self.capability_seen[index];
        let capability = &mut self.manifest.capabilities[index];
        match key {
            "resource" => {
                mark(seen, 1)?;
                capability.resource = BoundedText::new(parse_string(value)?)?
            }
            "kind" => {
                mark(seen, 2)?;
                capability.kind = match parse_string(value)? {
                    "ipc" => CapabilityKind::Ipc,
                    "shared-memory" => CapabilityKind::SharedMemory,
                    "file" => CapabilityKind::File,
                    "network" => CapabilityKind::Network,
                    "device" => CapabilityKind::Device,
                    "actor" => CapabilityKind::Actor,
                    "clock" => CapabilityKind::Clock,
                    _ => return Err(ManifestError::UnknownValue),
                }
            }
            "rights" => {
                mark(seen, 4)?;
                capability.rights = parse_rights(value)?
            }
            "required" => {
                mark(seen, 8)?;
                capability.required = parse_bool(value)?
            }
            _ => return Err(ManifestError::UnknownKey),
        }
        Ok(())
    }

    fn finish(mut self) -> Result<AppManifest, ManifestError> {
        if self.seen_root != 1 || self.seen_application != 7 {
            return Err(ManifestError::MissingField);
        }
        if self.manifest.schema != APP_MANIFEST_SCHEMA {
            return Err(ManifestError::UnsupportedSchema);
        }
        for seen in &self.capability_seen[..self.manifest.capability_count as usize] {
            if seen & 7 != 7 {
                return Err(ManifestError::MissingField);
            }
        }
        for (index, capability) in self.manifest.capabilities().enumerate() {
            if self
                .manifest
                .capabilities()
                .take(index)
                .any(|existing| existing.resource == capability.resource)
            {
                return Err(ManifestError::DuplicateKey);
            }
        }

        let runtime = &mut self.manifest.runtime;
        match runtime.restart_mode {
            RestartMode::Never => {
                if self.seen_runtime & 0b11_1100 != 0 {
                    return Err(ManifestError::InvalidRestartPolicy);
                }
                runtime.restart = RestartPolicy::NEVER
            }
            RestartMode::OnFailure | RestartMode::Always => {
                if self.seen_runtime & 0b11_1100 != 0b11_1100
                    || runtime.restart.max_restarts == 0
                    || runtime.restart.window_us == 0
                    || runtime.restart.initial_backoff_us == 0
                    || runtime.restart.max_backoff_us < runtime.restart.initial_backoff_us
                {
                    return Err(ManifestError::InvalidRestartPolicy);
                }
            }
        }
        Ok(self.manifest)
    }
}

fn mark(seen: &mut u8, bit: u8) -> Result<(), ManifestError> {
    if *seen & bit != 0 {
        Err(ManifestError::DuplicateKey)
    } else {
        *seen |= bit;
        Ok(())
    }
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in line.bytes().enumerate() {
        if byte == b'"' && !escaped {
            quoted = !quoted
        }
        if byte == b'#' && !quoted {
            return &line[..index];
        }
        escaped = byte == b'\\' && !escaped
    }
    line
}

fn parse_string(value: &str) -> Result<&str, ManifestError> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(ManifestError::InvalidString);
    }
    let inner = &value[1..value.len() - 1];
    if inner.is_empty()
        || inner
            .bytes()
            .any(|byte| byte == 0 || byte == b'\\' || byte == b'"')
    {
        return Err(ManifestError::InvalidString);
    }
    Ok(inner)
}

fn parse_bool(value: &str) -> Result<bool, ManifestError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ManifestError::InvalidBoolean),
    }
}

fn parse_u16(value: &str) -> Result<u16, ManifestError> {
    parse_u64(value)?
        .try_into()
        .map_err(|_| ManifestError::InvalidInteger)
}

fn parse_u64(value: &str) -> Result<u64, ManifestError> {
    if value.is_empty()
        || value.starts_with('_')
        || value.ends_with('_')
        || value.as_bytes().windows(2).any(|pair| pair == b"__")
    {
        return Err(ManifestError::InvalidInteger);
    }
    let mut parsed = 0u64;
    for byte in value.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return Err(ManifestError::InvalidInteger);
        }
        parsed = parsed
            .checked_mul(10)
            .and_then(|number| number.checked_add((byte - b'0') as u64))
            .ok_or(ManifestError::InvalidInteger)?
    }
    Ok(parsed)
}

fn parse_image(value: &str) -> Result<u128, ManifestError> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    if digits.is_empty() || digits.len() > 32 {
        return Err(ManifestError::InvalidInteger);
    }
    let image = u128::from_str_radix(digits, 16).map_err(|_| ManifestError::InvalidInteger)?;
    if image == 0 {
        Err(ManifestError::InvalidInteger)
    } else {
        Ok(image)
    }
}

fn parse_rights(value: &str) -> Result<CapabilityRights, ManifestError> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(ManifestError::InvalidRights);
    }
    let mut rights = CapabilityRights::NONE;
    for item in value[1..value.len() - 1].split(',') {
        let name = parse_string(item.trim()).map_err(|_| ManifestError::InvalidRights)?;
        let right = match name {
            "read" => CapabilityRights::READ,
            "write" => CapabilityRights::WRITE,
            "execute" => CapabilityRights::EXECUTE,
            "map" => CapabilityRights::MAP,
            "create" => CapabilityRights::CREATE,
            "send" => CapabilityRights::SEND,
            "receive" => CapabilityRights::RECEIVE,
            "delegate" => CapabilityRights::DELEGATE,
            _ => return Err(ManifestError::InvalidRights),
        };
        if rights.contains(right) {
            return Err(ManifestError::InvalidRights);
        }
        rights = rights.union(right)
    }
    if rights.is_empty() {
        Err(ManifestError::InvalidRights)
    } else {
        Ok(rights)
    }
}
