use ghostos_status::Status;
use ghostos_system_model::command::{ArgumentKind, ArgumentSpec, CommandSpec};

use crate::{
    Error,
    parser::{CommandCall, CommandRegistry, Value},
};

pub const MOUNT_ROUTE: u16 = 43;
pub const UNMOUNT_ROUTE: u16 = 44;
pub const SHOW_MOUNTS_ROUTE: u16 = 45;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageProtocol {
    PnfsV42,
    Smb311 { multichannel: bool, direct: bool },
    NvmeOfTcp,
    NvmeOfRoceV2,
    Iscsi,
    S3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCacheMode {
    Disabled,
    ReadThrough,
    CopyOnWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMountRequest<'a> {
    pub server: &'a str,
    pub logical: &'a str,
    pub protocol: StorageProtocol,
    pub cache: StorageCacheMode,
    pub read_only: bool,
    pub multichannel: bool,
    pub direct: bool,
    pub port: Option<u16>,
}

pub fn register_storage_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let nfs = flag("NFS")?;
    let smb = flag("SMB")?;
    let nvme = flag("NVME")?;
    let iscsi = flag("ISCSI")?;
    let s3 = flag("S3")?;
    let server = ArgumentSpec::new("SERVER", ArgumentKind::Text, true, false)
        .map_err(|_| Error::InvalidValue)?;
    let logical = ArgumentSpec::new("LOGICAL", ArgumentKind::Text, true, false)
        .map_err(|_| Error::InvalidValue)?;
    let cache = ArgumentSpec::new("CACHE", ArgumentKind::Text, false, false)
        .map_err(|_| Error::InvalidValue)?;
    let read_only = flag("READ_ONLY")?;
    let multichannel = flag("MULTICHANNEL")?;
    let direct = flag("DIRECT")?;
    let port = ArgumentSpec::new("PORT", ArgumentKind::Integer, false, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new(
            "MOUNT",
            &[
                nfs,
                smb,
                nvme,
                iscsi,
                s3,
                server,
                logical,
                cache,
                read_only,
                multichannel,
                direct,
                port,
            ],
        )
        .map_err(|_| Error::InvalidValue)?,
        crate::parser::RouteId::from_valid_raw(MOUNT_ROUTE),
    )?;
    let mount_id = ArgumentSpec::new("MOUNT", ArgumentKind::Integer, true, true)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("UNMOUNT", &[mount_id]).map_err(|_| Error::InvalidValue)?,
        crate::parser::RouteId::from_valid_raw(UNMOUNT_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-MOUNTS", &[]).map_err(|_| Error::InvalidValue)?,
        crate::parser::RouteId::from_valid_raw(SHOW_MOUNTS_ROUTE),
    )?;
    Ok(())
}

pub fn mount_request<'a>(command: &'a CommandCall) -> Result<StorageMountRequest<'a>, Status> {
    if command.route.raw() != MOUNT_ROUTE {
        return Err(Status::INVALID_ARGUMENT)
    }
    let server = command.get_text("SERVER").filter(|value| !value.is_empty()).ok_or(Status::INVALID_ARGUMENT)?;
    let logical = command.get_text("LOGICAL").filter(|value| !value.is_empty()).ok_or(Status::INVALID_ARGUMENT)?;
    let protocol = selected_protocol(command)?;
    let cache = match command.get_text("CACHE") {
        None => StorageCacheMode::ReadThrough,
        Some(value) if value.eq_ignore_ascii_case("NONE") => StorageCacheMode::Disabled,
        Some(value) if value.eq_ignore_ascii_case("COW") => StorageCacheMode::CopyOnWrite,
        Some(value) if value.eq_ignore_ascii_case("READ_THROUGH") => StorageCacheMode::ReadThrough,
        Some(_) => return Err(Status::INVALID_ARGUMENT),
    };
    let port = match command.get("PORT") {
        None => None,
        Some(Value::Integer(value)) => Some(u16::try_from(value).map_err(|_| Status::INVALID_ARGUMENT)?),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
    };
    Ok(StorageMountRequest {
        server,
        logical,
        protocol,
        cache,
        read_only: boolean(command.get("READ_ONLY"))?,
        multichannel: boolean(command.get("MULTICHANNEL"))?,
        direct: boolean(command.get("DIRECT"))?,
        port,
    })
}

fn selected_protocol(command: &CommandCall) -> Result<StorageProtocol, Status> {
    let selected = [
        ("NFS", command.get("NFS"), StorageProtocol::PnfsV42),
        (
            "SMB",
            command.get("SMB"),
            StorageProtocol::Smb311 {
                multichannel: boolean(command.get("MULTICHANNEL"))?,
                direct: boolean(command.get("DIRECT"))?,
            },
        ),
        (
            "NVME",
            command.get("NVME"),
            if boolean(command.get("DIRECT"))? {
                StorageProtocol::NvmeOfRoceV2
            } else {
                StorageProtocol::NvmeOfTcp
            },
        ),
        ("ISCSI", command.get("ISCSI"), StorageProtocol::Iscsi),
        ("S3", command.get("S3"), StorageProtocol::S3),
    ];
    let mut result = None;
    for (_, value, protocol) in selected {
        if value == Some(Value::Boolean(true)) {
            if result.is_some() {
                return Err(Status::INVALID_ARGUMENT)
            }
            result = Some(protocol)
        }
    }
    result.ok_or(Status::INVALID_ARGUMENT)
}

fn boolean(value: Option<Value>) -> Result<bool, Status> {
    match value {
        None => Ok(false),
        Some(Value::Boolean(value)) => Ok(value),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}

fn flag(name: &str) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, ArgumentKind::Boolean, false, false).map_err(|_| Error::InvalidValue)
}
