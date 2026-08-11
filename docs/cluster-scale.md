# Cluster scale target

SynOS's deterministic cluster control-plane target is 1,000 members. The
campaign covers 10, 100, and 1,000 nodes at each release boundary:

- discovery returns every member;
- one heartbeat per non-coordinator member reaches the coordinator;
- a one-tick transport converges in one tick;
- fencing removes one failed node from the running set and rejects its next
  heartbeat;
- recovery restores the node and accepts exactly one recovery heartbeat.

The fixture uses a one-coordinator heartbeat topology. This keeps control-plane
traffic at `32N - 28` bytes for `N` nodes: `4N` discovery bytes plus `28(N-1)`
heartbeat bytes. Membership and heartbeat metadata remain linear; the test
rejects more than `68N` bytes of resident control-plane metadata. A single
failure emits no fanout traffic; recovery emits one heartbeat, so failure
amplification is bounded by one recovery message.

Regression command:

```sh
cargo test -p synos-vm --test cluster cluster_scale_campaign_is_bounded_at_10_100_and_1000_nodes
```

The campaign is deterministic and does not depend on wall-clock timing or
cluster hardware. Implementation and coverage live in
[`virtual_machine/src/cluster.rs`](../virtual_machine/src/cluster.rs) and
[`virtual_machine/tests/cluster.rs`](../virtual_machine/tests/cluster.rs).
