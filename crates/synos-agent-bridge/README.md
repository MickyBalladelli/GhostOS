# synos-agent-bridge

`synos-agent-bridge` is the native boundary between an AI agent and
`syn-script`.

- `AgentBridge::tool_schemas` reflects the live command registry as bounded
  OpenAI-compatible JSON-Schema tools.
- `SingleUseCapabilityIssuer` derives a short-lived token from a broader parent
  capability, seals it to one agent and task scope, and rejects replay after one
  successful authorization.
- `RunMode::Sandbox` implements `RUN /SANDBOX`: all changes execute on a private
  SynFS CoW root and are discarded.
- `RunMode::Commit` uses the same private root, but publishes it atomically only
  when the script succeeds and the caller has supplied an approved token with
  write authority.

`AgentBridge::prepare` keeps the completed CoW transaction private so a caller
can inspect its typed output and staged files, then call `approve` or `discard`.
Approval publishes that exact validated root; it does not rerun the script.

Command handlers must make system changes through the supplied `CowSandbox`.
All consumers of agent capabilities must authorize them through
`SingleUseCapabilityIssuer::consume`; direct cryptographic verification cannot
enforce the replay ledger.
