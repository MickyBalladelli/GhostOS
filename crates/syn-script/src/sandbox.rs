use syn_shell::{
    Error as ShellError,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::CommandCall,
};
use synos_status::Status;
use synos_synfs::{FileVersion, ReadResult, SynFs, SynFsTransaction, TransactionCommit};
use synos_system_model::command::StructuredOutput;

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxDecision {
    Discard,
    Commit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxReceipt {
    pub base_generation: u64,
    pub staged_generation: u64,
    pub resulting_generation: u64,
    pub operations: u32,
    pub committed: bool,
}

/// Isolated SynFS CoW view for validating agent-generated operations.
///
/// Writes remain on a private root. Reads see the staged state, allowing an
/// executor to validate command results. Dropping or discarding the sandbox
/// restores the original root and reclaims abandoned blocks.
pub struct CowSandbox<'a, const MAX_BLOCKS: usize> {
    transaction: Option<SynFsTransaction<'a, MAX_BLOCKS>>,
    base_generation: u64,
}

pub trait SandboxCommandHandler<const MAX_BLOCKS: usize> {
    fn execute(
        &mut self,
        sandbox: &mut CowSandbox<'_, MAX_BLOCKS>,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<StructuredOutput, Status>;
}

/// Ordered command adapter between `ScriptEngine` and a CoW sandbox.
///
/// A command is executed exactly once during submission, then its typed result
/// is returned on the next poll. Only one pipeline stage can be pending.
pub struct SandboxExecutor<'sandbox, 'filesystem, Handler, const MAX_BLOCKS: usize> {
    sandbox: &'sandbox mut CowSandbox<'filesystem, MAX_BLOCKS>,
    handler: Handler,
    completion: Option<(ExecutionToken, Result<StructuredOutput, Status>)>,
    next_token: u64,
}

impl<'sandbox, 'filesystem, Handler, const MAX_BLOCKS: usize>
    SandboxExecutor<'sandbox, 'filesystem, Handler, MAX_BLOCKS>
where
    Handler: SandboxCommandHandler<MAX_BLOCKS>,
{
    pub const fn new(
        sandbox: &'sandbox mut CowSandbox<'filesystem, MAX_BLOCKS>,
        handler: Handler,
    ) -> Self {
        Self {
            sandbox,
            handler,
            completion: None,
            next_token: 1,
        }
    }

    pub fn handler(&self) -> &Handler {
        &self.handler
    }

    pub fn handler_mut(&mut self) -> &mut Handler {
        &mut self.handler
    }

    pub fn into_handler(self) -> Handler {
        self.handler
    }
}

impl<Handler, const MAX_BLOCKS: usize> CommandExecutor
    for SandboxExecutor<'_, '_, Handler, MAX_BLOCKS>
where
    Handler: SandboxCommandHandler<MAX_BLOCKS>,
{
    fn submit(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, ShellError> {
        if self.completion.is_some() {
            return Err(ShellError::AlreadyRunning);
        }
        let token = ExecutionToken::new(self.next_token).ok_or(ShellError::Capacity)?;
        self.next_token = self.next_token.checked_add(1).ok_or(ShellError::Capacity)?;
        let completion = self.handler.execute(self.sandbox, command, pipeline_input);
        self.completion = Some((token, completion));
        Ok(token)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        if !self
            .completion
            .as_ref()
            .is_some_and(|(expected, _)| *expected == token)
        {
            return Some(Err(Status::INVALID_ARGUMENT));
        }
        self.completion.take().map(|(_, completion)| completion)
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), ShellError> {
        if !self
            .completion
            .as_ref()
            .is_some_and(|(expected, _)| *expected == token)
        {
            return Err(ShellError::InvalidHandle);
        }
        self.completion = None;
        Ok(())
    }
}

impl<'a, const MAX_BLOCKS: usize> CowSandbox<'a, MAX_BLOCKS> {
    pub fn new(filesystem: &'a mut SynFs<MAX_BLOCKS>) -> Self {
        let base_generation = filesystem.generation();
        Self {
            transaction: Some(filesystem.transaction()),
            base_generation,
        }
    }

    pub const fn base_generation(&self) -> u64 {
        self.base_generation
    }

    pub fn staged_generation(&self) -> u64 {
        self.transaction()
            .map(|transaction| transaction.generation())
            .unwrap_or(self.base_generation)
    }

    pub fn operations(&self) -> u32 {
        self.transaction()
            .map(|transaction| transaction.operations())
            .unwrap_or(0)
    }

    pub fn lookup(&self, path: &str) -> Result<FileVersion, Error> {
        Ok(self.active()?.lookup(path)?)
    }

    pub fn read(&self, path: &str, destination: &mut [u8]) -> Result<ReadResult, Error> {
        Ok(self.active()?.read(path, destination)?)
    }

    pub fn write(&mut self, path: &str, contents: &[u8]) -> Result<FileVersion, Error> {
        Ok(self.active_mut()?.write(path, contents)?)
    }

    pub fn delete(&mut self, path: &str) -> Result<FileVersion, Error> {
        Ok(self.active_mut()?.delete(path)?)
    }

    pub fn finish(mut self, decision: SandboxDecision) -> Result<SandboxReceipt, Error> {
        let transaction = self.transaction.take().ok_or(Error::Inactive)?;
        let staged_generation = transaction.generation();
        let operations = transaction.operations();
        match decision {
            SandboxDecision::Discard => {
                drop(transaction);
                Ok(SandboxReceipt {
                    base_generation: self.base_generation,
                    staged_generation,
                    resulting_generation: self.base_generation,
                    operations,
                    committed: false,
                })
            }
            SandboxDecision::Commit => {
                let TransactionCommit {
                    generation,
                    operations,
                } = transaction.commit()?;
                Ok(SandboxReceipt {
                    base_generation: self.base_generation,
                    staged_generation,
                    resulting_generation: generation,
                    operations,
                    committed: true,
                })
            }
        }
    }

    fn transaction(&self) -> Option<&SynFsTransaction<'a, MAX_BLOCKS>> {
        self.transaction.as_ref()
    }

    fn active(&self) -> Result<&SynFsTransaction<'a, MAX_BLOCKS>, Error> {
        self.transaction().ok_or(Error::Inactive)
    }

    fn active_mut(&mut self) -> Result<&mut SynFsTransaction<'a, MAX_BLOCKS>, Error> {
        self.transaction.as_mut().ok_or(Error::Inactive)
    }
}
