use ghostos_status::Status;
use ghostos_system_model::command::StructuredOutput;

use crate::{
    Error,
    jobs::{JobId, JobOwner, JobPolicy, JobQueue},
    parser::{CommandCall, CommandRegistry, Program},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ExecutionToken(u64);

impl ExecutionToken {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

pub trait CommandExecutor {
    fn submit(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error>;

    fn poll(
        &mut self,
        token: ExecutionToken,
    ) -> Option<Result<StructuredOutput, Status>>;

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error>;
}

pub enum InterpreterEvent {
    Started,
    Pending,
    Submitted(JobId),
    Complete(StructuredOutput),
    Failed(Status),
    Cancelled,
}

/// Non-blocking command and structured-pipeline state machine.
pub struct Interpreter {
    program: Option<Program>,
    stage: usize,
    pending: Option<ExecutionToken>,
    pipeline_output: Option<StructuredOutput>,
}

impl Interpreter {
    pub const fn new() -> Self {
        Self {
            program: None,
            stage: 0,
            pending: None,
            pipeline_output: None,
        }
    }

    pub fn start<
        const COMMANDS: usize,
        const JOBS: usize,
        Executor: CommandExecutor,
    >(
        &mut self,
        line: &str,
        registry: &CommandRegistry<COMMANDS>,
        executor: &mut Executor,
        jobs: &mut JobQueue<JOBS>,
        owner: JobOwner,
        policy: JobPolicy,
    ) -> Result<InterpreterEvent, Error> {
        if self.program.is_some() {
            return Err(Error::AlreadyRunning)
        }
        let program = registry.parse(line)?;
        if program.background {
            return Ok(InterpreterEvent::Submitted(
                jobs.submit(owner, program, policy)?,
            ))
        }
        self.start_program(program, executor)
    }

    pub fn start_program<Executor: CommandExecutor>(
        &mut self,
        mut program: Program,
        executor: &mut Executor,
    ) -> Result<InterpreterEvent, Error> {
        if self.program.is_some() {
            return Err(Error::AlreadyRunning)
        }
        program.background = false;
        let first = program.stage(0).ok_or(Error::InvalidSyntax)?;
        let token = executor.submit(first, None)?;
        self.program = Some(program);
        self.stage = 0;
        self.pending = Some(token);
        self.pipeline_output = None;
        Ok(InterpreterEvent::Started)
    }

    pub fn poll<Executor: CommandExecutor>(
        &mut self,
        executor: &mut Executor,
    ) -> Result<InterpreterEvent, Error> {
        let program = self.program.ok_or(Error::NoActiveCommand)?;
        let token = self.pending.ok_or(Error::NoActiveCommand)?;
        let Some(completion) = executor.poll(token) else {
            return Ok(InterpreterEvent::Pending)
        };
        self.pending = None;
        let output = match completion {
            Ok(output) => output,
            Err(status) => {
                self.clear();
                return Ok(InterpreterEvent::Failed(status))
            }
        };

        let next_stage = self.stage + 1;
        if let Some(next) = program.stage(next_stage) {
            let next_token = match executor.submit(next, Some(&output)) {
                Ok(token) => token,
                Err(error) => {
                    self.clear();
                    return Err(error)
                }
            };
            self.pipeline_output = Some(output);
            self.stage = next_stage;
            self.pending = Some(next_token);
            Ok(InterpreterEvent::Pending)
        } else {
            self.clear();
            Ok(InterpreterEvent::Complete(output))
        }
    }

    pub fn cancel<Executor: CommandExecutor>(
        &mut self,
        executor: &mut Executor,
    ) -> Result<InterpreterEvent, Error> {
        let token = self.pending.ok_or(Error::NoActiveCommand)?;
        executor.cancel(token)?;
        self.clear();
        Ok(InterpreterEvent::Cancelled)
    }

    pub const fn is_running(&self) -> bool {
        self.program.is_some()
    }

    fn clear(&mut self) {
        self.program = None;
        self.stage = 0;
        self.pending = None;
        self.pipeline_output = None
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::register_filesystem_commands;

    struct PendingExecutor {
        cancelled: bool,
    }

    impl CommandExecutor for PendingExecutor {
        fn submit(
            &mut self,
            _command: CommandCall,
            _pipeline_input: Option<&StructuredOutput>,
        ) -> Result<ExecutionToken, Error> {
            Ok(ExecutionToken::new(1).expect("valid pending token"))
        }

        fn poll(&mut self, _token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
            None
        }

        fn cancel(&mut self, _token: ExecutionToken) -> Result<(), Error> {
            self.cancelled = true;
            Ok(())
        }
    }

    #[test]
    fn cancellation_clears_a_pending_wildcard_command() {
        let mut registry = CommandRegistry::<16>::new();
        register_filesystem_commands(&mut registry).expect("filesystem commands");
        let program = registry.parse("TYPE /data/*.txt").expect("wildcard command");
        let mut executor = PendingExecutor { cancelled: false };
        let mut interpreter = Interpreter::new();

        assert!(matches!(
            interpreter.start_program(program, &mut executor),
            Ok(InterpreterEvent::Started)
        ));
        assert!(interpreter.is_running());
        assert!(matches!(
            interpreter.cancel(&mut executor),
            Ok(InterpreterEvent::Cancelled)
        ));
        assert!(!interpreter.is_running());
        assert!(executor.cancelled);
    }
}
