# Declarative system configuration

`ghostos-declarative` parses bounded `System.toml` input without heap allocation. The
existing `[system]`, `[[service]]`, `[[capability]]`, and `[network]` sections remain
valid. Cluster policy is added with these sections:

```toml
[system]
schema = 1
revision = 12

[cluster]
id = 0x1234
name = "prod"
discovery = "hybrid"
membership = "automatic"
admission = "attested"
heartbeat_period_us = 1000000
missed_heartbeat_limit = 5

[cluster.quorum]
voting_members = 3
required_votes = 2
read_only_without_quorum = true

[cluster.security]
require_signed_commits = true
require_mutual_identity = true
require_attestation = true
trust_root = 0x55

[cluster.resources]
cpu_limit = 64
memory_limit_bytes = 1099511627776
network_limit_mbps = 10000

[[cluster.transport]]
name = "lan"
kind = "ethernet"
endpoint = "10.0.0.1:7000"
priority = 10

[cluster.federation]
enabled = true
allow_remote_workloads = false
lease_ttl_us = 3600000000
max_leases = 8

[[cluster.node-override]]
node = 2
heartbeat_period_us = 500000
transport = "ethernet"
```

`ClusterSpec::safe_defaults` supplies safe single-node defaults when the cluster
section is absent. `SystemSpec::validate` rejects invalid quorum, heartbeat,
transport, federation, trust-root, and override values before activation.

`ConfigurationDiff::between` returns changed areas plus affected node and service
identifiers. `ClusterConfigurationManager` stages signed updates, checks the
expected previous revision, enforces optional maintenance windows, runs runtime
health checks, requires quorum acknowledgement, commits through a GhostFS
checkpoint, and keeps bounded rollback history. `ClusterSpec::effective_for`
applies a node override over the base policy.
