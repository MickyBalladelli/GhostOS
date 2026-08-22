# Control-plane sharding and recovery

`ghostos-storaged::ClusterShardCoordinator` owns bounded shard placement for
cluster metadata, capability indexes, package catalogs, audit streams, and
placement decisions. Existing service state remains authoritative. The
coordinator adds routing, ownership fencing, and recovery around that state.

| Namespace | Preserved contract |
| --- | --- |
| Cluster metadata | Linearizable generation-checked updates |
| Capability index | Linearizable revocation and authorization updates |
| Package catalog | Snapshot reads over immutable package objects |
| Audit stream | Append-only committed indexes |
| Placement decision | Linearizable generation-checked decisions |

Writes require the current leader, current membership term, current node
epoch, and a majority of the shard replicas. A move is prepare, target
acknowledgement, then commit. The old primary remains a replica until commit.
Term changes fence old leaders; failed nodes are fenced before promotion or
reconciliation. Reconciliation only returns a shard to `Stable` after every
replica is available.

The regression records bounded event evidence and computes a deterministic
fingerprint from the committed shard generations, indexes, ownership, and
recovery events. Replay must produce identical evidence:

```sh
cargo test -p ghostos-storaged --test sharding -- --nocapture
```

The test covers shard movement, same-term split-brain rejection, deterministic
rebalancing, node loss, promotion, rejoin fencing, and stable recovery.
