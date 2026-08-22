use ghostos_status::Status;
use ghostos_system_model::command::{ArgumentKind, ArgumentSpec, CommandSpec, StructuredOutput};

use crate::{
    Error,
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const RUST_ROUTE: u16 = 56;
pub const SHOW_RUST_JOBS_ROUTE: u16 = 57;
pub const TOOLCHAIN_ROUTE: u16 = 58;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RustAction {
    Check,
    Build,
    Run,
    Test,
    Doc,
}

impl RustAction {
    pub fn parse(value: &str) -> Result<Self, Status> {
        match value {
            value if value.eq_ignore_ascii_case("CHECK") => Ok(Self::Check),
            value if value.eq_ignore_ascii_case("BUILD") => Ok(Self::Build),
            value if value.eq_ignore_ascii_case("RUN") => Ok(Self::Run),
            value if value.eq_ignore_ascii_case("TEST") => Ok(Self::Test),
            value if value.eq_ignore_ascii_case("DOC") => Ok(Self::Doc),
            _ => Err(Status::INVALID_ARGUMENT),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustRequest<'a> {
    pub action: RustAction,
    pub manifest: Option<&'a str>,
    pub binary: Option<&'a str>,
    pub target: Option<&'a str>,
    pub release: bool,
    pub locked: bool,
    pub offline: bool,
    pub json: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolchainAction {
    Install,
    Select,
    Update,
    Rollback,
}

impl ToolchainAction {
    pub fn parse(value: &str) -> Result<Self, Status> {
        match value {
            value if value.eq_ignore_ascii_case("INSTALL") => Ok(Self::Install),
            value if value.eq_ignore_ascii_case("SELECT") => Ok(Self::Select),
            value if value.eq_ignore_ascii_case("UPDATE") => Ok(Self::Update),
            value if value.eq_ignore_ascii_case("ROLLBACK") => Ok(Self::Rollback),
            _ => Err(Status::INVALID_ARGUMENT),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainRequest<'a> {
    pub action: ToolchainAction,
    pub stage: Option<&'a str>,
    pub target: Option<&'a str>,
    pub bundle: Option<&'a str>,
    pub key: Option<&'a str>,
    pub json: bool,
}

pub trait RustSource {
    fn rust(&mut self, request: RustRequest<'_>) -> Result<StructuredOutput, Status>;
    fn rust_jobs(&mut self, json: bool) -> Result<StructuredOutput, Status>;
    fn toolchain(&mut self, request: ToolchainRequest<'_>) -> Result<StructuredOutput, Status>;
}

pub fn register_rust_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let action = text("ACTION", true, true)?;
    let manifest = text("MANIFEST", false, false)?;
    let binary = text("BIN", false, false)?;
    let target = text("TARGET", false, false)?;
    let release = flag("RELEASE")?;
    let locked = flag("LOCKED")?;
    let offline = flag("OFFLINE")?;
    let json = flag("JSON")?;
    registry.register(
        CommandSpec::new(
            "RUST",
            &[action, manifest, binary, target, release, locked, offline, json],
        )
        .map_err(|_| Error::InvalidValue)?,
        route(RUST_ROUTE),
    )?;

    registry.register(
        CommandSpec::new("SHOW-RUST-JOBS", &[json]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_RUST_JOBS_ROUTE),
    )?;

    let stage = text("STAGE", false, false)?;
    let bundle = text("BUNDLE", false, false)?;
    let key = text("KEY", false, false)?;
    registry.register(
        CommandSpec::new("TOOLCHAIN", &[action, stage, target, bundle, key, json])
            .map_err(|_| Error::InvalidValue)?,
        route(TOOLCHAIN_ROUTE),
    )
}

pub fn execute_rust_command<Source: RustSource>(
    source: &mut Source,
    command: CommandCall,
) -> Result<StructuredOutput, Status> {
    match command.route.raw() {
        RUST_ROUTE => source.rust(rust_request(&command)?),
        SHOW_RUST_JOBS_ROUTE => source.rust_jobs(boolean(command.get("JSON"))?),
        TOOLCHAIN_ROUTE => source.toolchain(toolchain_request(&command)?),
        _ => Err(Status::NOT_FOUND),
    }
}

fn rust_request<'a>(command: &'a CommandCall) -> Result<RustRequest<'a>, Status> {
    let action = RustAction::parse(command.get_text("ACTION").ok_or(Status::INVALID_ARGUMENT)?)?;
    Ok(RustRequest {
        action,
        manifest: command.get_text("MANIFEST"),
        binary: command.get_text("BIN"),
        target: command.get_text("TARGET"),
        release: boolean(command.get("RELEASE"))?,
        locked: boolean(command.get("LOCKED"))?,
        offline: boolean(command.get("OFFLINE"))?,
        json: boolean(command.get("JSON"))?,
    })
}

fn toolchain_request<'a>(command: &'a CommandCall) -> Result<ToolchainRequest<'a>, Status> {
    Ok(ToolchainRequest {
        action: ToolchainAction::parse(
            command.get_text("ACTION").ok_or(Status::INVALID_ARGUMENT)?,
        )?,
        stage: command.get_text("STAGE"),
        target: command.get_text("TARGET"),
        bundle: command.get_text("BUNDLE"),
        key: command.get_text("KEY"),
        json: boolean(command.get("JSON"))?,
    })
}

fn text(name: &str, required: bool, positional: bool) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, ArgumentKind::Text, required, positional)
        .map_err(|_| Error::InvalidValue)
}

fn flag(name: &str) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, ArgumentKind::Boolean, false, false)
        .map_err(|_| Error::InvalidValue)
}

fn route(raw: u16) -> RouteId {
    RouteId::from_valid_raw(raw)
}

fn boolean(value: Option<Value>) -> Result<bool, Status> {
    match value {
        None => Ok(false),
        Some(Value::Boolean(value)) => Ok(value),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}
