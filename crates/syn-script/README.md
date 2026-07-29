# syn-script

`syn-script` is SynOS's heap-free native command-procedure engine. It compiles
bounded DCL-style source into fixed-capacity statements and drives the existing
typed `syn-shell` command registry cooperatively.

```text
$ SET SYMBOL NODE = 7
$ DEFINE /PROCESS /FILE INPUT = "data/events.rms"
$ SHOW NODE ${NODE} | SELECT /STATE=READY
$ SET NOON
$ IF FAILURE THEN SHOW STATUS $STATUS
$ ATTENUATE ADMIN AS READER /RIGHTS=READ /TRANSPORT=CXL /EXPIRES=5000000
$ EXIT $STATUS
```

Command arguments are validated against `CommandSpec` before dispatch. A
pipeline stage receives the previous stage's `StructuredOutput`, never a text
stream. The `wire` module encodes the same arguments and output fields into a
checksummed `synos-ipc` shared-region payload; only its `SharedBuffer`
descriptor crosses the IPC ring.

Logical-name verbs support process, job, group, system, and cluster scopes plus
file, device, and IPC-channel targets. `AuthorizedLogicalNames` routes those
verbs through `CapabilityLogicalNames`, so script syntax cannot bypass the
kernel capability and logical-name ACL checks.

Capability symbols hold cryptographic tokens. `ATTENUATE` adds a Macaroon-style
caveat with reduced rights, transport, expiry, and optional subject. It cannot
mint or widen authority.

`ScriptEngine::status()` is the script's `$STATUS`. Odd values mean success.
Procedures stop on a failed statement by default. `SET NOON` or
`ON ERROR THEN CONTINUE` keeps execution active so `IF FAILURE THEN ...` can
handle the condition; `SET ON` restores failure propagation.
