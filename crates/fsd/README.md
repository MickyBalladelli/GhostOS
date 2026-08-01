# synos-fsd

`synos-fsd` is the Ring 3 owner of a SynFS volume. It handles bounded IPC
requests, daemon-issued process and file capabilities, CoW transactions,
checkpoints, snapshots, mount records, and garbage collection.

The IPC request uses one kernel-validated shared buffer. Paths are UTF-8;
rename uses `old-path\0new-path`; listing buffers use an optional NUL-terminated
prefix and are overwritten with newline-separated paths.
