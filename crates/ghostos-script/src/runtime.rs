use ghostos_shell::{
    interpreter::{CommandExecutor, Interpreter, InterpreterEvent},
    parser::CommandRegistry,
};
use ghostos_auth::CapabilityCaveat;
use ghostos_status::{IntoStatus, Status};
use ghostos_system_model::command::StructuredOutput;

use crate::{
    Error, MAX_SCRIPT_STATEMENTS,
    context::{LogicalNameControl, ScriptContext, SymbolValue},
    parser::{ErrorPolicy, ExitStatus, Script, StatementKind, SymbolLiteral},
};

pub enum ScriptEvent {
    Pending,
    PipelineComplete(StructuredOutput),
    StatementFailed(Status),
    Failed(Status),
    Complete(Status),
    Cancelled,
}

/// Cooperative native command-procedure interpreter.
///
/// Pipeline stages are parsed by `ghostos-shell`, so command arguments are typed
/// before dispatch and each stage receives the prior `StructuredOutput`.
pub struct ScriptEngine<const STATEMENTS: usize = MAX_SCRIPT_STATEMENTS> {
    script: Option<Script<STATEMENTS>>,
    program_counter: usize,
    status: Status,
    error_policy: ErrorPolicy,
    pipeline: Interpreter,
    pipeline_pending: bool,
}

impl<const STATEMENTS: usize> ScriptEngine<STATEMENTS> {
    pub const fn new() -> Self {
        Self {
            script: None,
            program_counter: 0,
            status: Status::NORMAL,
            error_policy: ErrorPolicy::Exit,
            pipeline: Interpreter::new(),
            pipeline_pending: false,
        }
    }

    pub fn start(&mut self, script: Script<STATEMENTS>) -> Result<(), Error> {
        if self.script.is_some() {
            return Err(Error::AlreadyRunning);
        }
        self.script = Some(script);
        self.program_counter = 0;
        self.status = Status::NORMAL;
        self.error_policy = ErrorPolicy::Exit;
        self.pipeline_pending = false;
        Ok(())
    }

    pub fn poll<
        const COMMANDS: usize,
        const SYMBOLS: usize,
        Executor: CommandExecutor,
        LogicalNames: LogicalNameControl,
    >(
        &mut self,
        registry: &CommandRegistry<COMMANDS>,
        executor: &mut Executor,
        context: &mut ScriptContext<LogicalNames, SYMBOLS>,
    ) -> Result<ScriptEvent, Error> {
        if self.script.is_none() {
            return Err(Error::Inactive);
        }
        if self.pipeline_pending {
            return self.poll_pipeline(executor);
        }

        loop {
            let Some(script) = self.script else {
                return Err(Error::Inactive)
            };
            let Some(statement) = script.statement(self.program_counter) else {
                let status = self.status;
                self.clear();
                return Ok(ScriptEvent::Complete(status));
            };
            self.program_counter += 1;

            if !statement.condition.matches(self.status) {
                continue;
            }

            match statement.kind {
                StatementKind::SetErrorPolicy(policy) => {
                    self.error_policy = policy;
                }
                StatementKind::Exit(exit) => {
                    let status = match exit {
                        ExitStatus::Current => self.status,
                        ExitStatus::Value(status) => status,
                    };
                    self.status = status;
                    self.clear();
                    return Ok(ScriptEvent::Complete(status));
                }
                StatementKind::SetSymbol { name, value } => {
                    let value = match value {
                        SymbolLiteral::Boolean(value) => SymbolValue::Boolean(value),
                        SymbolLiteral::Integer(value) => SymbolValue::Integer(value),
                        SymbolLiteral::Status(value) => SymbolValue::Status(value),
                        SymbolLiteral::Text(value) => SymbolValue::Text(value),
                    };
                    if let Err(error) = context.symbols.define(name.as_str(), value) {
                        return Ok(self.statement_error(error.status()));
                    }
                    self.status = Status::NORMAL;
                }
                StatementKind::DefineLogical(definition) => {
                    let identity = context.identity;
                    let scope = match definition.scope.resolve(
                        identity.process,
                        identity.job,
                        identity.group,
                    ) {
                        Ok(scope) => scope,
                        Err(error) => return Ok(self.statement_error(error.status())),
                    };
                    if let Err(error) = context.logical_names.define(
                        scope,
                        definition.name.as_str(),
                        definition.target,
                    ) {
                        return Ok(self.statement_error(error.status()));
                    }
                    self.status = Status::NORMAL;
                }
                StatementKind::DeleteLogical(deletion) => {
                    let identity = context.identity;
                    let scope =
                        match deletion
                            .scope
                            .resolve(identity.process, identity.job, identity.group)
                        {
                            Ok(scope) => scope,
                            Err(error) => return Ok(self.statement_error(error.status())),
                        };
                    if let Err(error) = context.logical_names.delete(scope, deletion.name.as_str())
                    {
                        return Ok(self.statement_error(error.status()));
                    }
                    self.status = Status::NORMAL;
                }
                StatementKind::Attenuate(attenuation) => {
                    let source = context.symbols.get(attenuation.source.as_str());
                    let Some(SymbolValue::Capability(source)) = source else {
                        return Ok(self.statement_error(Error::InvalidCapability.status()));
                    };
                    let caveat = CapabilityCaveat {
                        subject: attenuation.subject,
                        rights: attenuation.rights,
                        transports: attenuation
                            .transports
                            .unwrap_or_else(|| source.effective_transports()),
                        expires_at_us: attenuation.expires_at_us,
                    };
                    if let Err(error) = context.attenuate(
                        attenuation.source.as_str(),
                        attenuation.destination.as_str(),
                        caveat,
                    ) {
                        return Ok(self.statement_error(error.status()));
                    }
                    self.status = Status::NORMAL;
                }
                StatementKind::Pipeline(line) => {
                    let expanded = match context.symbols.expand(line.as_str(), self.status) {
                        Ok(expanded) => expanded,
                        Err(error) => return Ok(self.statement_error(error.status())),
                    };
                    let program = match registry.parse(expanded.as_str()) {
                        Ok(program) if !program.background => program,
                        Ok(_) => {
                            return Ok(
                                self.statement_error(ghostos_shell::Error::InvalidSyntax.status())
                            );
                        }
                        Err(error) => return Ok(self.statement_error(error.status())),
                    };
                    match self.pipeline.start_program(program, executor) {
                        Ok(InterpreterEvent::Started) => {
                            self.pipeline_pending = true;
                            return Ok(ScriptEvent::Pending);
                        }
                        Ok(_) => {
                            return Ok(
                                self.statement_error(ghostos_shell::Error::InvalidSyntax.status())
                            );
                        }
                        Err(error) => return Ok(self.statement_error(error.status())),
                    }
                }
            }
        }
    }

    pub fn cancel<Executor: CommandExecutor>(
        &mut self,
        executor: &mut Executor,
    ) -> Result<ScriptEvent, Error> {
        if self.script.is_none() {
            return Err(Error::Inactive);
        }
        if self.pipeline_pending {
            self.pipeline.cancel(executor)?;
        }
        self.clear();
        Ok(ScriptEvent::Cancelled)
    }

    pub const fn status(&self) -> Status {
        self.status
    }

    pub const fn is_running(&self) -> bool {
        self.script.is_some()
    }

    pub const fn program_counter(&self) -> usize {
        self.program_counter
    }

    fn poll_pipeline<Executor: CommandExecutor>(
        &mut self,
        executor: &mut Executor,
    ) -> Result<ScriptEvent, Error> {
        match self.pipeline.poll(executor)? {
            InterpreterEvent::Pending => Ok(ScriptEvent::Pending),
            InterpreterEvent::Complete(output) => {
                self.pipeline_pending = false;
                self.status = output.status();
                if !self.status.is_success() && self.error_policy == ErrorPolicy::Exit {
                    let status = self.status;
                    self.clear();
                    Ok(ScriptEvent::Failed(status))
                } else {
                    Ok(ScriptEvent::PipelineComplete(output))
                }
            }
            InterpreterEvent::Failed(status) => {
                self.pipeline_pending = false;
                Ok(self.statement_error(status))
            }
            InterpreterEvent::Cancelled => {
                self.clear();
                Ok(ScriptEvent::Cancelled)
            }
            InterpreterEvent::Started | InterpreterEvent::Submitted(_) => {
                Err(Error::Shell(ghostos_shell::Error::InvalidSyntax))
            }
        }
    }

    fn statement_error(&mut self, status: Status) -> ScriptEvent {
        self.status = status;
        if self.error_policy == ErrorPolicy::Exit {
            self.clear();
            ScriptEvent::Failed(status)
        } else {
            ScriptEvent::StatementFailed(status)
        }
    }

    fn clear(&mut self) {
        self.script = None;
        self.pipeline_pending = false;
    }
}

impl<const STATEMENTS: usize> Default for ScriptEngine<STATEMENTS> {
    fn default() -> Self {
        Self::new()
    }
}
