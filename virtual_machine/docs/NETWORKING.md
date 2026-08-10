# VM networking

The VM has two emulated NICs: e1000 and virtio-net. Choose one backend in
`VmConfig.network`.

## Deterministic test network

Use `DeterministicVmNetwork` for repeatable tests. It creates one bounded L2
segment. Each VM gets two unique MAC addresses. Add a `DhcpServerConfig` to
enable the bounded DHCP server, or pass `None` to test no-server behavior.

```rust
let network = DeterministicVmNetwork::new(Some(DhcpServerConfig::default()))?;
let config = VmConfig {
    network: NetworkBackendConfig::DeterministicShared { network: network.clone() },
    dhcp_server: None,
    ..VmConfig::default()
};
let first = Vm::try_with_config(config.clone())?;
let second = Vm::try_with_config(config)?;
```

The fixture supports bounded pools, reservations, lease expiry, carrier loss,
administrative disable, packet loss, queue saturation, and backend disconnect.
Use it for the two-VM integration and fault matrix; it does not model an
external router or DNS resolver.

## User-mode/NAT

`NetworkBackendConfig::UserNat { bind, peer }` wraps Ethernet frames in a
bounded UDP transport. The peer is a host-side gateway or test adapter. NAT
does not provide a bridged L2 identity, inbound connections without an explicit
forward, or proof of external DHCP behavior.

## Bridged networking

`NetworkBackendConfig::Bridged { interface }` binds the VM to a host Ethernet
interface. The Linux backend uses AF_PACKET. It needs host privileges and a
carrier-connected interface. The backend does not create a bridge and does not
provide DHCP; configure the host bridge and DHCP service separately.

## Sample two-VM flow

1. Create one `DeterministicVmNetwork` with a DHCP pool.
2. Construct two VMs with `DeterministicShared` and the same network handle.
3. Poll the fixture until each VM receives an offer, then send a request and
   poll again for the ACK.
4. Check `SHOW INTERFACES` for different MAC and IPv4 addresses.
5. Run `PING <peer-address> /COUNT=3 /IPV4`, then open a TCP listener and
   connect to it from the other VM.

Expected interface output contains separate identity and link fields:

```text
interface: eth0
mac: 02:53:59:4f:53:56
admin: enabled
link: up
ipv4: 10.5.0.100/24
dhcp: bound
```

The exact MAC and address depend on attachment order and pool configuration.

## Troubleshooting

`link: down` means the physical/backend carrier is down. DHCP must stop
transmitting and stays blocked until carrier returns. `dhcp: init` means the
interface is enabled but no DHCP transaction has completed; check the server,
pool, queue, and packet capture. A disabled NIC reports administrative down and
must not be confused with carrier loss.

Snapshots do not carry backend queues, carrier state, leases, routes, DNS,
neighbors, or DHCP timers. Rebuild the same topology, recover the network
service lease record, and re-establish link before sending traffic. See the
[snapshot state inventory](SNAPSHOT_STATE_INVENTORY.md).
