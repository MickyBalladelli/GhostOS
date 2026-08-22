use ghostos_status::Status;
use ghostos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputValue, StructuredOutput,
};

use crate::{
    Error,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const SHOW_FIREWALL_ROUTE: u16 = 43;
pub const SET_FIREWALL_ROUTE: u16 = 44;
pub const FIREWALL_POLICY_PATH: &str = "SYS$SYSTEM:FIREWALL.POLICY;1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirewallView {
    pub policy_version: u64,
    pub rule_count: u64,
    pub active_connections: u64,
    pub dropped_packets: u64,
    pub allowed_packets: u64,
}

pub trait FirewallSource {
    fn show_firewall(&mut self, version: Option<u64>) -> Result<FirewallView, Status>;
    fn set_firewall_rule(&mut self, rule: &str) -> Result<FirewallView, Status>;
}

pub fn register_firewall_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let version = ArgumentSpec::new("VERSION", ArgumentKind::Integer, false, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("SHOW-FIREWALL", &[version]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_FIREWALL_ROUTE),
    )?;
    let rule = ArgumentSpec::new("RULE", ArgumentKind::Text, true, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("SET-FIREWALL", &[rule]).map_err(|_| Error::InvalidValue)?,
        route(SET_FIREWALL_ROUTE),
    )
}

pub struct FirewallExecutor<Source, const CAPACITY: usize = 16> {
    source: Source,
    completions: [Option<Result<StructuredOutput, Status>>; CAPACITY],
}

impl<Source, const CAPACITY: usize> FirewallExecutor<Source, CAPACITY> {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            completions: [const { None }; CAPACITY],
        }
    }

    pub const fn source(&self) -> &Source { &self.source }
    pub const fn source_mut(&mut self) -> &mut Source { &mut self.source }
}

impl<Source: FirewallSource, const CAPACITY: usize> CommandExecutor
    for FirewallExecutor<Source, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .completions
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let completion = match command.route.raw() {
            SHOW_FIREWALL_ROUTE => {
                let version = unsigned(command.get("VERSION"))?;
                self.source.show_firewall(version).and_then(firewall_output)
            }
            SET_FIREWALL_ROUTE => {
                let rule = match command.get("RULE") {
                    Some(Value::Text(value)) => value,
                    _ => return Err(Error::InvalidValue),
                };
                self.source.set_firewall_rule(rule.as_str()).and_then(firewall_output)
            }
            _ => Err(Status::NOT_FOUND),
        };
        self.completions[slot] = Some(completion);
        ExecutionToken::new((slot + 1) as u64).ok_or(Error::InvalidHandle)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        self.completions.get_mut(token.raw().checked_sub(1)? as usize)?.take()
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let completion = self.completions.get_mut(token.raw().checked_sub(1).ok_or(Error::InvalidHandle)? as usize).ok_or(Error::InvalidHandle)?;
        *completion = None;
        Ok(())
    }
}

fn route(raw: u16) -> RouteId { RouteId::from_valid_raw(raw) }

fn unsigned(value: Option<Value>) -> Result<Option<u64>, Error> {
    match value {
        Some(Value::Integer(value)) if value >= 0 => Ok(Some(value as u64)),
        None => Ok(None),
        _ => Err(Error::InvalidValue),
    }
}

pub fn firewall_output(view: FirewallView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    output.insert("policy-path", OutputValue::Text(
        ghostos_system_model::command::OutputText::new(FIREWALL_POLICY_PATH)
            .map_err(|_| Status::NO_SPACE)?,
    )).map_err(|_| Status::NO_SPACE)?;
    output.insert("policy-version", OutputValue::Unsigned(view.policy_version)).map_err(|_| Status::NO_SPACE)?;
    output.insert("rule-count", OutputValue::Unsigned(view.rule_count)).map_err(|_| Status::NO_SPACE)?;
    output.insert("active-connections", OutputValue::Unsigned(view.active_connections)).map_err(|_| Status::NO_SPACE)?;
    output.insert("dropped-packets", OutputValue::Unsigned(view.dropped_packets)).map_err(|_| Status::NO_SPACE)?;
    output.insert("allowed-packets", OutputValue::Unsigned(view.allowed_packets)).map_err(|_| Status::NO_SPACE)?;
    Ok(output)
}
