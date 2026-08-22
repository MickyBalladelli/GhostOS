use core::fmt::Write;

use ghostos_observability::{
    AuditQuery, CapabilityTrace, CapabilityTraceStage, Level, TraceEvent,
};
use ghostos_status::Status;
use ghostos_system_model::command::{ArgumentKind, ArgumentSpec, CommandSpec};

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
    let auth_action = qualifier("AUTH_ACTION")?;
    let identity = qualifier("IDENTITY")?;
    let caller = qualifier("CALLER")?;
    registry.register(
        CommandSpec::new(
            "ANALYZE-AUDIT",
            &[
                since,
                before,
                capability,
                node,
                status,
                auth_action,
                identity,
                caller,
            ],
        )
            .map_err(|_| Error::InvalidValue)?,
        RouteId::from_valid_raw(ANALYZE_AUDIT_ROUTE),
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
    source.analyze(query_from_call(command)?, &mut |event| {
        mark_shell_output(event);
        visitor(event)
    })
}

pub fn mark_shell_output(event: TraceEvent) {
    let Some(trace) = CapabilityTrace::from_event(event) else {
        return
    };
    if let Some(shell_trace) = CapabilityTrace::new(
        trace.domain,
        CapabilityTraceStage::ShellOutput,
        trace.capability,
        trace.operation,
    ) {
        shell_trace.emit(Level::Info)
    }
}

pub fn render_capability_trace(
    event: TraceEvent,
) -> Result<crate::Text<{ crate::render::MAX_RENDERED_OUTPUT_BYTES }>, Error> {
    let trace = CapabilityTrace::from_event(event).ok_or(Error::InvalidValue)?;
    mark_shell_output(event);
    let mut output = crate::Text::empty();
    write!(
        &mut output,
        "CAPABILITY domain={} stage={} capability={} operation={}\n",
        trace.domain.name(),
        trace.stage.name(),
        trace.capability,
        trace.operation,
    )
    .map_err(|_| Error::Capacity)?;
    Ok(output)
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
        auth_action: unsigned(command.get("AUTH_ACTION"))?,
        identity: unsigned(command.get("IDENTITY"))?,
        caller: unsigned(command.get("CALLER"))?,
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
