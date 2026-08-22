# ghostos-storaged

Ring 3 enterprise remote storage service model.

The crate provides bounded, capability-checked mount management for pNFS
4.1/4.2, SMB 3.1.1 multi-channel and SMB Direct, NVMe-oF TCP/RoCEv2, iSCSI,
and S3. Remote protocol adapters submit asynchronous I/O requests to the
daemon and return completions through its fixed queues.

`CowCache` provides read-through and private Copy-on-Write caching. S3 bodies
are delivered as borrowed buffers from the `ghostos-netd` side through
`NetworkBufferStream`, so the storage service does not copy network buffers.

Mounts use `SYS$STORAGE:<logical>/path` paths. The shell registration accepts
structured commands such as:

```text
MOUNT /NFS /SERVER=isilon.local:/data /LOGICAL=DATA_POOL /CACHE=COW
```

`MountCatalog` encodes persistent definitions for the GhostFS CoW state file
`SYS$SYSTEM:MOUNTS.DAT;1`.
