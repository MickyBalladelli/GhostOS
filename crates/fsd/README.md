# ghostos-fsd

`ghostos-fsd` is the Ring 3 owner of a GhostFS volume. It handles bounded IPC
requests, daemon-issued process and file capabilities, CoW transactions,
checkpoints, snapshots, mount records, and garbage collection.

Startup activates a fixed logical namespace with writable `/`, `/logs`, `/data`,
and `/tmp`, plus read-only `/packages` GhostFS mounts. Host
Ext4, FAT32, and NTFS partitions can only be added with the boot-issued host
mount authority; those mounts are always read-only and use generation-checked
mount capabilities.

The IPC request uses one kernel-validated shared buffer. Paths are UTF-8;
rename uses `old-path\0new-path`; listing buffers use an optional NUL-terminated
prefix and are overwritten with newline-separated paths.

Directories have typed metadata and immediate-child listing. `Mkdir`, `Rmdir`,
and `Link` use the same bounded shared-buffer protocol. `Rmdir` requires delete,
write, and administration authority, rejects mount roots, and removes only empty
directories. Its response reports directory size, version, removal generation,
and whether storage reclamation is pending. GhostFS stores hard-link metadata,
quotas, free-space counters, and per-volume limits in the persistent volume
format.

The shell exposes `RMDIR path` and the `RD` alias. Paths are resolved from the
active default directory; `/`, version selectors, wildcards, non-directories,
non-empty directories, and the active default directory are rejected.

`Delete` accepts a path with an optional GhostFS version selector. `DELETE path`
and `DELETE path;0` remove the latest live version; `DELETE path;N` removes
only version `N`. The response reports the deleted version, file type,
remaining link count, and whether the shared data is still reachable. A hard
link deletion removes only that directory entry.

Wildcard paths use `*` for zero or more characters in one component, `?` for
one character, and bracket classes such as `[a-z]` or `[!a]`. Matching is
case-sensitive, includes hidden names, and never crosses `/`. A backslash
escapes a wildcard; quoted wildcard characters are escaped by the shell.
Patterns are resolved from the active default directory before expansion.
Matches are canonical, duplicate-free, sorted, and bounded. No match returns
`NOT_FOUND`; malformed patterns return `INVALID_ARGUMENT`; scan or output
limits return `NO_SPACE`.

`DIRECTORY`/`LS`, `TYPE`, `DELETE`, and `SHOW LINKS` accept wildcard paths.
`DELETE` uses the latest live version unless a numeric `;N` selector is added
to the pattern. A wildcard with `;N` matches only names retaining live version
`N`; it never falls back to the latest version or returns retained versions
from other selectors. `;0` means latest. Delete stops at the first failed
match after earlier matches were committed. `TYPE` separates matched files
with a path header. `LINK`, `MKDIR`,
`CREATE`, `RMDIR`, `SET DEFAULT`, and `CD` reject wildcard paths because their
target mapping is ambiguous or unsafe. Wildcards in version selectors are not
valid. `LINK source;N target` links the selected retained version as a fixed
hard-link target; later source versions do not change that link. Link source
wildcards and target selectors remain rejected. `SHOW LINKS path;N` reports
links for only the selected version's shared data.

Future path operations reuse this contract. `RENAME` requires an unambiguous
source-to-literal-target mapping. `COPY` permits wildcard sources only when a
literal existing directory can receive each result. `PURGE` follows `DELETE`
version selection and partial-commit reporting. Read-only metadata expansion
is safe when each returned record remains bounded; protection expansion also
requires explicit per-object authority. Any ambiguous wildcard target is
rejected before mutation.

## Shell validation workflow

```text
DIRECTORY /data
MKDIR /data/work
SET DEFAULT /data/work
CREATE "daily note"
TYPE "daily note"
SHOW DEFAULT
```

Expected structured results include `operation`, `path`, `type`, `size`,
`version`, and `link-count`. Directory results add typed `entry-N-name`,
`entry-N-type`, `entry-N-size`, `entry-N-version`, and `entry-N-link-count`
fields plus `next` for pagination. `TYPE` adds `content`, `content-bytes`,
`encoding`, and `truncated`; default-directory commands return
`default-directory`.

The validation coverage checks parser rejection, capability authorization,
bounded shared buffers, continuation pages, status mapping, and restart
persistence. The ignored QEMU test uses `GHOSTOS_RUN_QEMU_TESTS=1` and checks the
same listing, mkdir, create, type, and default-directory sequence on a booted
guest.

## Delete examples

```text
DELETE /data/note
DELETE /data/note;2
LINK /data/note /data/backup
DELETE /data/note
SHOW LINKS /data/backup
DELETE /data/backup
```

The first command removes the latest live version. `;N` removes only version
`N`; older versions are not silently selected. Deleting one hard-link name
keeps the shared data reachable through the other name. Deleting the final
name leaves no live directory entry, and garbage collection can reclaim the
unreachable data. Root paths, directories, missing versions, malformed
selectors, read-only mounts, and unauthorized paths fail with stable status
messages.
