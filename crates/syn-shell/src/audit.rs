use synos_observability::{AuditQuery, TraceEvent};
use synos_status::Status;
use synos_system_model::command::{ArgumentKind, ArgumentSpec, CommandSpec};

use crate::{
    Error,
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const ANALYZE_AUDIT_ROUTE: u16 = 4;

pub trait AuditSource {
    fn analyze(
        &mut self,
        query: AuditQuery,
        visitor: &mut dyn FnMut(TraceEvent),
    ) -> Result<usize, Status>;
}

pub fn register_audit_command<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let since = qualifier("SINCE")?;
    let before = qualifier("BEFORE")?;
    let capability = qualifier("CAPABILITY")?;
    let node = qualifier("NODE")?;
    let status = qualifier("STATUS")?;
    registry.register(
        CommandSpec::new("ANALYZE-AUDIT", &[since, before, capability, node, status])
            .map_err(|_| Error::InvalidValue)?,
        RouteId::new(ANALYZE_AUDIT_ROUTE).expect("nonzero route"),
    )
}

pub fn execute_audit(
    source: &mut impl AuditSource,
    command: CommandCall,
    visitor: &mut dyn FnMut(TraceEvent),
) -> Result<usize, Status> {
    if command.route.raw() != ANALYZE_AUDIT_ROUTE {
        return Err(Status::NOT_FOUND);
    }
    source.analyze(query_from_call(command)?, visitor)
}

pub fn query_from_call(command: CommandCall) -> Result<AuditQuery, Status> {
    Ok(AuditQuery {
        since: unsigned(command.get("SINCE"))?,
        before: unsigned(command.get("BEFORE"))?,
        capability: unsigned(command.get("CAPABILITY"))?,
        node: unsigned(command.get("NODE"))?
            .map(u32::try_from)
            .transpose()
            .map_err(|_| Status::INVALID_ARGUMENT)?,
        status: unsigned(command.get("STATUS"))?
            .map(u32::try_from)
            .transpose()
            .map_err(|_| Status::INVALID_ARGUMENT)?,
    })
}

fn qualifier(name: &str) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, ArgumentKind::Integer, false, false).map_err(|_| Error::InvalidValue)
}

fn unsigned(value: Option<Value>) -> Result<Option<u64>, Status> {
    match value {
        Some(Value::Integer(value)) if value >= 0 => Ok(Some(value as u64)),
        None => Ok(None),
        _ => Err(Status::INVALID_ARGUMENT),
    }
}
