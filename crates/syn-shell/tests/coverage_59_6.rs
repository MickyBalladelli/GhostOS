use syn_shell::{
    Error, Text,
    editor::{EditorAction, Key, LineEditor},
    interpreter::{CommandExecutor, ExecutionToken, Interpreter, InterpreterEvent},
    jobs::{JobOwner, JobPolicy, JobQueue, JobState, WorkerId},
    parser::{CommandRegistry, RouteId},
};
use synos_status::Status;
use synos_system_model::command::{CommandSpec, StructuredOutput};

fn registry() -> CommandRegistry<4> {
    let mut registry = CommandRegistry::new();
    registry
        .register(CommandSpec::new("FIRST", &[]).unwrap(), RouteId::new(1).unwrap())
        .unwrap();
    registry
        .register(CommandSpec::new("SECOND", &[]).unwrap(), RouteId::new(2).unwrap())
        .unwrap();
    registry
}

#[test]
fn line_editor_handles_utf8_cursor_history_and_bounded_input() {
    let mut editor = LineEditor::<2>::new();
    editor.handle(Key::Character('é')).unwrap();
    editor.handle(Key::Character('x')).unwrap();
    editor.handle(Key::Left).unwrap();
    editor.handle(Key::Backspace).unwrap();
    assert_eq!(editor.line(), "x");
    assert_eq!(editor.cursor(), 0);
    editor.handle(Key::End).unwrap();
    assert_eq!(editor.handle(Key::Enter).unwrap(), EditorAction::Submit(Text::new("x").unwrap()));
    editor.replace_line("second").unwrap();
    editor.handle(Key::Enter).unwrap();
    editor.handle(Key::HistoryPrevious).unwrap();
    assert_eq!(editor.line(), "second");
    assert_eq!(editor.handle(Key::Tab).unwrap(), EditorAction::Complete);
    let mut full = LineEditor::<2>::new();
    assert_eq!(full.replace_line(&"a".repeat(syn_shell::MAX_LINE_BYTES + 1)), Err(Error::LineTooLong));
}

struct Executor {
    next: u64,
    pending: Option<(ExecutionToken, Result<StructuredOutput, Status>)>,
    submissions: usize,
}

impl Executor {
    fn new() -> Self {
        Self {
            next: 1,
            pending: None,
            submissions: 0,
        }
    }
}

impl CommandExecutor for Executor {
    fn submit(
        &mut self,
        _command: syn_shell::parser::CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let token = ExecutionToken::new(self.next).unwrap();
        self.next += 1;
        self.submissions += 1;
        self.pending = Some((token, Ok(StructuredOutput::new(Status::NORMAL))));
        Ok(token)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        if self.pending.as_ref().is_some_and(|entry| entry.0 == token) {
            self.pending.take().map(|entry| entry.1)
        } else {
            Some(Err(Status::INVALID_ARGUMENT))
        }
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        if self.pending.take().is_some_and(|entry| entry.0 == token) {
            Ok(())
        } else {
            Err(Error::InvalidHandle)
        }
    }
}

#[test]
fn interpreter_runs_pipelines_and_background_jobs_with_leases() {
    let registry = registry();
    let mut executor = Executor::new();
    let mut interpreter = Interpreter::new();
    let mut jobs = JobQueue::<4>::new();
    let owner = JobOwner::new(7).unwrap();
    assert!(matches!(
        interpreter
            .start("FIRST | SECOND", &registry, &mut executor, &mut jobs, owner, JobPolicy::immediate())
            .unwrap(),
        InterpreterEvent::Started
    ));
    assert!(matches!(interpreter.poll(&mut executor).unwrap(), InterpreterEvent::Pending));
    assert!(matches!(
        interpreter.poll(&mut executor).unwrap(),
        InterpreterEvent::Complete(output) if output.status() == Status::NORMAL
    ));
    assert_eq!(executor.submissions, 2);

    let submitted = interpreter
        .start("FIRST &", &registry, &mut executor, &mut jobs, owner, JobPolicy::immediate())
        .unwrap();
    assert!(matches!(submitted, InterpreterEvent::Submitted(_)));
    let worker = WorkerId::new(3).unwrap();
    let lease = jobs.claim(worker, 0, 10).unwrap().unwrap();
    assert_eq!(jobs.info(lease.id).unwrap().state, JobState::Running);
    assert_eq!(jobs.renew(lease, 1, 10).unwrap().worker, worker);
    assert_eq!(jobs.finish(lease, Status::NORMAL), Ok(JobState::Completed));
    assert_eq!(jobs.reap(lease.id, owner), Ok(()));
}

#[test]
fn job_lease_expiry_requeues_and_owner_checks_are_enforced() {
    let registry = registry();
    let program = registry.parse("FIRST").unwrap();
    let owner = JobOwner::new(9).unwrap();
    let other = JobOwner::new(10).unwrap();
    let mut jobs = JobQueue::<2>::new();
    let id = jobs
        .submit(
            owner,
            program,
            JobPolicy {
                max_attempts: 2,
                ..JobPolicy::immediate()
            },
        )
        .unwrap();
    let _lease = jobs.claim(WorkerId::new(1).unwrap(), 0, 5).unwrap().unwrap();
    assert_eq!(jobs.cancel(id, other), Err(Error::NotOwner));
    assert_eq!(jobs.recover_expired(5), 1);
    assert_eq!(jobs.info(id).unwrap().state, JobState::Queued);
    assert_eq!(jobs.claim(WorkerId::new(2).unwrap(), 5, 5).unwrap().unwrap().id, id);
}
