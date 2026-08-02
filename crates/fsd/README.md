# synos-fsd

`synos-fsd` is the Ring 3 owner of a SynFS volume. It handles bounded IPC
requests, daemon-issued process and file capabilities, CoW transactions,
checkpoints, snapshots, mount records, and garbage collection.

Startup activates a fixed logical namespace with `/` and read-only
`/packages`, plus writable `/logs`, `/data`, and `/tmp` SynFS mounts. Host
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
and whether storage reclamation is pending. SynFS stores hard-link metadata,
quotas, free-space counters, and per-volume limits in the persistent volume
format.

The shell exposes `RMDIR path` and the `RD` alias. Paths are resolved from the
active default directory; `/`, version selectors, wildcards, non-directories,
non-empty directories, and the active default directory are rejected.

`Delete` accepts a path with an optional SynFS version selector. `DELETE path`
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
to the pattern, and stops at the first failed match after earlier matches were
committed. `TYPE` separates matched files with a path header. `LINK`, `MKDIR`,
`CREATE`, `RMDIR`, `SET DEFAULT`, and `CD` reject wildcard paths because their
target mapping is ambiguous or unsafe. Wildcards in version selectors are not
valid.
