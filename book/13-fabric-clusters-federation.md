# 13. Fabric, Clustering, Federation, and Time

GhostOS treats local memory, CXL memory, remote Layer-2 memory, VRAM, and selected storage as one controlled resource graph.

## `ghostos-fabric`

The fabric crate discovers CXL endpoints through PCI CXL DVSECs, validates Type-3 devices, and programs HDM decoder component registers through isolated MMIO. Generation-checked leases allocate aligned RAM or VRAM ranges.

The global address map resolves:

```text
local RAM | CXL | remote memory | VRAM
```

The address is unified. The transport is not hidden. Local and CXL mappings can be direct and coherent; Layer-2 memory uses remote page requests and a tiered NUMA cache.

## DSM

Software DSM moves 4 KiB pages over raw Ethernet payloads using EtherType `0x88b5`. Requests reject stale epochs and malformed fragments. A writer invalidates remote sharers before acquiring exclusive access.

Page movement follows a simple rule:

```text
fetch -> validate epoch -> map read
write -> invalidate sharers -> acquire write lease -> publish
```

## Cluster membership

Cluster members have IDs, states, heartbeat records, quorum, epochs, and lifecycle actions. Cluster create, join, leave, rejoin, invitation, attestation, fencing, and recovery all have explicit states.

The system rejects:

- duplicate node IDs;
- stale epochs;
- expired or revoked invitations;
- protocol mismatch;
- no quorum;
- split-brain writes;
- unsafe nodes not yet isolated.

## Failure and fencing

When a node fails:

1. heartbeat detection advances membership state;
2. old leases become stale;
3. transport isolation is confirmed;
4. DLM and DSM ownership can be reassigned;
5. mirrored storage and workloads recover;
6. the old node may rejoin with a fresh epoch.

## Federation

Federated clusters discover peers through signed, expiring announcements. They trade bounded CPU, RAM, and VRAM leases. Borrowed work runs inside a `BlindMicroSilo` that cannot see host process trees, foreign GhostFS mounts, sockets, or federation controls.

Revocation has a deadline and epoch. A lending cluster can force a borrowed workload to stop or lose its resource.

## Load balancing and mesh

`ghostos-balancerd` handles active-active actor placement and cache-aware leases. `ghostos-mesh` handles edge-to-cloud discovery, advertisements, dynamic cluster membership, offload, and CoW delta reconciliation.

Mesh advertisements are signed and expiring. They carry cluster/node identity,
protocol versions, capabilities, and ordered endpoints across CXL, Ethernet,
wireless, 5G, loopback, tunnels, NAT, relay, and offline routes. A bounded
topology graph records zones, reachability, latency, bandwidth, transport,
route, and MTU. Failed endpoints rotate with capped exponential backoff, and
offline advertisements remain inspectable.

## Time

`ghostos-time-sync` keeps monotonic and synchronized clock contracts explicit. Time affects leases, heartbeats, deadlines, expiry, inference checkpoints, and replay. Tests use fake clocks and bounded jumps rather than sleeping and hoping.

## Easy example: cluster recovery

```text
node-2 misses heartbeat
  -> mark suspect
  -> epoch 8 becomes invalid
  -> isolate NIC/CXL
  -> fence node-2
  -> move mirror and leases
  -> continue safe reads
  -> rejoin node-2 at epoch 9
```

The old node is not “mostly trusted.” It is stale until the protocol says it is safe again.
