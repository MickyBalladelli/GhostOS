# ghostos-auditd

`ghostos-auditd` is the bounded Ring 3 security audit daemon for installed
GhostOS packages.

Feed adapters normalize RustSec, OSV, and CVE records into
`AdvisoryCatalog`. Each record is keyed by an immutable GhostOS package or
payload `ContentId`. `AuditDaemon::poll` scans a caller-selected number of
packages, then resumes at its cursor on the next poll. Findings are fixed
capacity and contain the advisory source, identifier, severity, package hash,
and matched hash kind.

`ObsolescenceRegistry` stores bounded package metadata findings for deprecated,
unmaintained, or out-of-date binaries and drivers. Each finding keeps its node,
package name, content hash, and installed/latest versions so capability-scoped
inspection can provide local or cluster `SHOW OBSOLETE` summaries.

Withdrawn advisories remain in the catalog so feed updates are deterministic,
but are not reported. No heap, filesystem access, network access, or blocking
work is performed by the daemon; those operations belong to capability-scoped
feed adapters and the service supervisor.
