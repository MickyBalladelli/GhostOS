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
cursor, mode, selection, and modification state. The bottom three rows show
the file-name and current-mode banner, the available commands for the active
mode, and the status line.

The editor accepts bounded UTF-8 text, preserves trailing newlines, redraws
after a resize event, and keeps the session open after capacity, I/O, or save
conflict errors. Successful shell results use stable operation, path, version,
and size fields.
