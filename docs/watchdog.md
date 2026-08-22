# Watchdog diagnostics

GhostOS always enforces service and CPU liveness checks. Watchdog diagnostic
messages are hidden by default so normal login and shell output stay usable.
Fencing still happens when diagnostics are off.

## Turn messages on

Run this in the GhostOS shell:

```text
WATCHDOG STATUS
WATCHDOG ON
```

Use `WATCHDOG OFF` to hide the messages again. `WATCHDOG STATUS` reports the
current setting and the service timeout. The setting lasts until reboot.

The command is available in first-run setup and after login. It requires the
GhostOS shell service; it does not bypass login authorization.

## Message definitions

`watchdog fenced hung service role=N for supervisor recovery` means the
service's heartbeat was missing for more than five seconds. GhostOS clears that
service's ready state so the supervisor can recover it. `N` is the service
role number below.

`watchdog offlined stalled CPU N` means a CPU stopped reporting its scheduler
heartbeat. GhostOS removes that CPU from normal scheduling.

These messages describe recovery actions. They do not mean that a user
command failed. Several messages at once usually mean the VM or scheduler
was paused long enough to trip multiple heartbeat deadlines.

## Service roles

| Role | Service |
| ---: | --- |
| 1 | `ghostos-init` |
| 2 | `ghostos-fsd` |
| 3 | `ghostos-storaged` |
| 4 | `ghostos-netd` |
| 5 | `ghostos-logd` |
| 6 | `ghostos-auditd` |
| 7 | `ghostos-authd` |
| 8 | `ghostos-pkgd` |
| 9 | `ghostos-shell` |
| 10 | `ghostos-pcid` |
| 11 | `ghostos-ahcid` |
| 12 | `ghostos-nvmed` |
| 13 | `ghostos-ethernetd` |
| 14 | `ghostos-logind` |

