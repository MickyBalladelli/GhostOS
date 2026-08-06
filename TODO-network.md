# Network TODO

## Network settings commands

- [x] Add `SHOW NETWORK` to display hostname and bounded interface/route summary counts.
- [x] Add `SHOW INTERFACES` for bounded per-interface output.
- [x] Add `SHOW ROUTES` for bounded route-table output.
- [x] Add `SET HOSTNAME` with validation and capability checks.
- [x] Add `SET INTERFACE` to change an interface address, gateway, MTU, and enabled state.
- [x] Add `SET ROUTE` to add or replace a route with destination, gateway, interface, and metric.
- [x] Add typed network command routes, qualifiers, structured output, and stable status mappings.
- [x] Add network command help entries and shell help aliases.
- [x] Connect network commands to the declarative configuration model and runtime reconfiguration service.
- [x] Stage, health-check, commit, and safely roll back network changes.
- [x] Persist accepted network configuration as a versioned SynFS configuration through the existing configuration runtime boundary.
- [x] Require the correct network-administration capability for mutations.
- [x] Add shell, parser, authorization, persistence, rollback, and runtime integration tests.

## DHCP

- [x] Add a capability-gated DHCP client service for IPv4 address assignment.
- [x] Implement bounded DHCP discover, offer, request, and acknowledgement handling.
- [x] Bind DHCP client traffic to a selected Ethernet interface.
- [x] Validate transaction IDs, client MAC address, lease timers, server identifier, subnet mask, gateway, DNS servers, and routes.
- [x] Support lease renewal, rebinding, expiry, release, link-down, and restart recovery.
- [x] Apply a DHCP lease atomically through the network configuration runtime.
- [x] Preserve the previous static configuration when DHCP fails or the lease expires.
- [x] Expose DHCP state and lease details through `SHOW INTERFACES`.
- [x] Add `SET INTERFACE ... /DHCP` and an explicit static-address mode.
- [x] Add bounded retry and backoff behavior for unavailable DHCP servers.
- [x] Add firewall rules and capability checks for DHCP client traffic.
- [x] Add deterministic DHCP server fixtures and tests for malformed packets, conflicting offers, renewals, expiry, and rollback.
