# Watchdog diagnostics

SynOS always enforces service and CPU liveness checks. Watchdog diagnostic
messages are hidden by default so normal login and shell output stay usable.
Fencing still happens when diagnostics are off.

## Turn messages on

Run this in the SynOS shell:

```text
WATCHDOG STATUS
WATCHDOG ON
```

Use `WATCHDOG OFF` to hide the messages again. `WATCHDOG STATUS` reports the
current setting and the service timeout. The setting lasts until reboot.

The command is available in first-run setup and after login. It requires the
SynOS shell service; it does not bypass login authorization.

## Message definitions

`watchdog fenced hung service role=N for supervisor recovery` means the
service's heartbeat was missing for more than five seconds. SynOS clears that
service's ready state so the supervisor can recover it. `N` is the service
role number below.

`watchdog offlined stalled CPU N` means a CPU stopped reporting its scheduler
heartbeat. SynOS removes that CPU from normal scheduling.

These messages describe recovery actions. They do not mean that a user
command failed. Several messages at once usually mean the VM or scheduler
was paused long enough to trip multiple heartbeat deadlines.

## Service roles

| Role | Service |
| ---: | --- |
| 1 | `synos-init` |
| 2 | `synos-fsd` |
| 3 | `synos-storaged` |
| 4 | `synos-netd` |
| 5 | `synos-logd` |
| 6 | `synos-auditd` |
| 7 | `synos-authd` |
| 8 | `synos-pkgd` |
| 9 | `synos-shell` |
| 10 | `synos-pcid` |
| 11 | `synos-ahcid` |
| 12 | `synos-nvmed` |
| 13 | `synos-ethernetd` |
| 14 | `synos-logind` |

