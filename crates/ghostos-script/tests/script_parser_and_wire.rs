// Inventory: coverage_59_6.rs (legacy roadmap section 59).
use ghostos_script::{
    Condition, Error, ErrorPolicy, Script, StatementKind, SymbolTable, SymbolValue,
    context::ExecutionIdentity,
    sandbox::{CowSandbox, SandboxDecision},
    wire::{decode_request, encode_request},
};
use ghostos_shell::{Text, parser::{CommandRegistry, RouteId}};
use ghostos_ipc::SharedRegionId;
use ghostos_status::Status;
use ghostos_system_model::command::{ArgumentKind, ArgumentSpec, CommandSpec, StructuredOutput};
use ghostos_ghostfs::SynFs;

#[test]
fn script_parser_handles_conditions_symbols_comments_and_limits() {
    let script = Script::<8>::compile(
        "# comment\n$ SET SYMBOL COUNT = 3\nIF $STATUS THEN SET NOON\nDEFINE /PROCESS DATA = value\nEXIT 1",
    )
    .expect("compile script");
    assert_eq!(script.len(), 4);
    assert!(matches!(script.statement(0).unwrap().kind, StatementKind::SetSymbol { .. }));
    assert_eq!(script.statement(1).unwrap().condition, Condition::Success);
    assert!(matches!(script.statement(2).unwrap().kind, StatementKind::DefineLogical(_)));
    assert!(matches!(script.statement(3).unwrap().kind, StatementKind::Exit(_)));
    assert!(Script::<1>::compile("FIRST\nSECOND").is_err());
    assert!(matches!(
        Script::<8>::compile("IF $STATUS MAYBE EXIT"),
        Err(Error::InvalidCondition)
    ));

    let mut symbols = SymbolTable::<2>::new();
    symbols.define("COUNT", SymbolValue::Integer(3)).unwrap();
    symbols
        .define("TEXT", SymbolValue::Text(Text::new("hello").unwrap()))
        .unwrap();
    let expanded = symbols.expand("${TEXT}:$STATUS:${COUNT}", Status::NORMAL).unwrap();
    assert!(expanded.as_str().starts_with("hello:"));
    symbols.delete("COUNT").unwrap();
    assert_eq!(symbols.get("count"), None);
    assert_eq!(ExecutionIdentity::new(0, None, None), None);
}

#[test]
fn script_wire_round_trip_preserves_typed_calls_and_rejects_tampering() {
    let mut registry = CommandRegistry::<2>::new();
    let argument = ArgumentSpec::new(
        "VALUE",
        ArgumentKind::Text,
        true,
        true,
    )
    .unwrap();
    registry
        .register(CommandSpec::new("ECHO", &[argument]).unwrap(), RouteId::new(41).unwrap())
        .unwrap();
    let call = registry.parse("ECHO hello").unwrap().stage(0).unwrap();
    let region = SharedRegionId::new(3).unwrap();
    let mut mapping = [0; 512];
    let envelope = encode_request(
        call,
        Some(&StructuredOutput::new(Status::NORMAL)),
        region,
        32,
        99,
        Some(7),
        &mut mapping,
    )
    .unwrap();
    let decoded = decode_request(envelope, region, &mapping).unwrap();
    assert_eq!(decoded.route, RouteId::new(41).unwrap());
    assert_eq!(decoded.command.as_str(), "ECHO");
    assert!(decoded.pipeline_input.is_some());
    assert_eq!(decoded.delegated_capability, Some(7));

    mapping[32] ^= 1;
    assert!(matches!(decode_request(envelope, region, &mapping), Err(Error::WireCorrupt)));
}

#[test]
fn cow_sandbox_discard_and_commit_isolate_script_file_changes() {
    let mut filesystem = SynFs::<64>::new();
    filesystem.write("/state", b"base").unwrap();
    {
        let mut sandbox = CowSandbox::new(&mut filesystem);
        sandbox.write("/state", b"discarded").unwrap();
        let receipt = sandbox.finish(SandboxDecision::Discard).unwrap();
        assert!(!receipt.committed);
        assert!(receipt.operations > 0);
    }
    let mut contents = [0; 4];
    filesystem.read("/state", &mut contents).unwrap();
    assert_eq!(&contents, b"base");

    let mut sandbox = CowSandbox::new(&mut filesystem);
    sandbox.write("/state", b"committed").unwrap();
    let receipt = sandbox.finish(SandboxDecision::Commit).unwrap();
    assert!(receipt.committed);
    let mut contents = [0; 9];
    filesystem.read("/state", &mut contents).unwrap();
    assert_eq!(&contents, b"committed");
}

#[test]
fn conditions_match_success_and_failure_statuses() {
    assert!(Condition::Success.matches(Status::NORMAL));
    assert!(Condition::Failure.matches(Status::ACCESS_DENIED));
    assert!(!Condition::Success.matches(Status::ACCESS_DENIED));
    assert_eq!(ErrorPolicy::Continue, ErrorPolicy::Continue);
}
