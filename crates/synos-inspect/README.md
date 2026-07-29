# synos-inspect

Heap-free, capability-scoped diagnostics for SynOS.

The crate supplies detailed memory/fabric, SynFS/storage, CPU, user, session,
and process snapshots. `ShellInspectionSource` connects those snapshots to:

- `SHOW MEMORY [/CLUSTER]`
- `SHOW DISK [/CLUSTER]`
- `SHOW CPU [/CLUSTER]`
- `SHOW USERS [/CLUSTER]`
- `SHOW PROCESS [pid]`

Local capabilities see their home node and their own activity. Cluster views
require `CAP_AUDIT_WORLD`. Capabilities are expiry- and revocation-epoch
checked before a provider is sampled.
