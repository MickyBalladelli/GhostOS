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
and `Link` use the same bounded shared-buffer protocol. SynFS stores hard-link
metadata, quotas, free-space counters, and per-volume limits in the persistent
volume format.
