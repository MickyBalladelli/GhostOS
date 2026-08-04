# syn-shell

The shell exposes the bounded full-screen file editor through EDIT and its
EDT alias.

## EDIT workflow

EDIT path opens the latest live version. EDIT path;N opens retained version N.
Saving never mutates the source; it publishes a new SynFS version. The editor
rejects a stale latest-version save with a conflict status and asks whether to
publish the buffer as another version.

Ctrl-S saves, Ctrl-Z saves and exits, Ctrl-X discards and exits, and Ctrl-C
cancels. Escape enters command mode:

- I inserts at the cursor.
- S saves.
- E saves and exits.
- Q discards and exits, with a prompt when modified.
- Y copies the selection, X cuts it, and P pastes it.

Arrow keys move the cursor. Shift plus arrows, Home, or End selects text.
Backspace and Delete remove selected text or join adjacent lines. The bottom
terminal row is a status line showing file name, size, version, line count,
cursor, mode, selection, and modification state. The bottom two rows show the
status line followed by the available commands for the active mode.

The editor accepts bounded UTF-8 text, preserves trailing newlines, redraws
after a resize event, and keeps the session open after capacity, I/O, or save
conflict errors. Successful shell results use stable operation, path, version,
and size fields.

## Filesystem shell workflow

```text
DIRECTORY /data
MKDIR /data/work
SET DEFAULT /data/work
CREATE "daily note"
TYPE "daily note"
SHOW DEFAULT
```

Created objects return `operation`, `path`, `type`, `size`, `version`, and
`link-count`. `DIRECTORY` returns typed `entry-N-*` fields and a numeric `next`
continuation when more entries remain. `TYPE` also returns `content`,
`content-bytes`, `encoding`, and `truncated`. `SET DEFAULT`, `CD`, `SHOW
DEFAULT`, and `PWD` return `default-directory`.

Missing paths return `NOT_FOUND`. Malformed paths, unknown qualifiers, extra
arguments, and wildcards on create or default-directory commands return
`INVALID_ARGUMENT` before filesystem I/O.

Wildcard commands use the same contract. A valid pattern with no visible
matches returns `NOT_FOUND`; malformed wildcard grammar returns
`INVALID_PATTERN`. If expansion or command execution stops after matches were
processed, the command returns `PARTIAL_MATCH` with `match-count`,
`processed-count`, `failed-count`, `failure-status`, and `partial` fields.
Cancellation returns `CANCELLED` and sets `cancelled` while preserving the
bounded work counts. `DIRECTORY`, `TYPE`, `DELETE`, and `SHOW LINKS` all stop
at the first operation failure; `DELETE` keeps already-completed deletions and
reports them as partial.

`UPTIME` displays the time since boot as `days, HH:MM:SS`. Structured output
also includes the microsecond uptime and each human-readable time component.
