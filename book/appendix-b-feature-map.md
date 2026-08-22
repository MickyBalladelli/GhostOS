# Appendix B. Feature Map

The root `TODO.md` numbers the project’s feature program. This map compresses those 58 roadmap areas into memorable groups.

| TODO | Feature | Remember it as |
| ---: | --- | --- |
| 1 | Core kernel architecture and GhostFS | boot, memory, filesystem |
| 2 | Legacy PC hardware and dual boot | PCI, drivers, compatibility |
| 3 | Linux pain-point solutions | clean service boundaries |
| 4 | OpenVMS integration | locks, names, records, status |
| 5 | Hardware fabric and clustering | CXL, DSM, leases |
| 6 | LLM enablement | model memory and KV cache |
| 7 | Target platforms and emulation | platform evidence |
| 8 | Architectural risk mitigations | COW, epochs, RCU, prefetch |
| 9 | Authentication, authorization, identity | personas and capabilities |
| 10 | Federation and cross-cluster sandboxing | borrow safely |
| 11 | Diagnostics, observability, auditing | explain the system |
| 12 | Rust toolchain and runtime ecosystem | build and ABI |
| 13 | Native interactive shell | typed commands |
| 14 | User-space async networking | packet ownership |
| 15 | Service isolation and fault recovery | restart one service |
| 16 | Pure-Rust compute and GPU abstraction | tensors without copies |
| 17 | Package distribution | immutable artifacts |
| 18 | Volume management and disaster recovery | pools, backup, restore |
| 19 | Power and hardware lifecycle | drain, quiesce, detach |
| 20 | Cluster topology visualization | see heat and latency |
| 21 | Cross-platform client SDKs | portable clients |
| 22 | Remote console and desktop | terminal and display |
| 23 | Mobile and remote security gateways | passkeys and scoped tokens |
| 24 | Application model and runtimes | manifests and restart |
| 25 | Application data and RMS | records and locks |
| 26 | High-level AI pipelines | agents and checkpoints |
| 27 | Web applications and microservices | HTTP and gRPC |
| 28 | Native command scripting | typed DCL procedures |
| 29 | Embedded/Rhai and Wasm | constrained automation |
| 30 | Agent orchestration | deterministic tools |
| 31 | Agent-native script bridge | grant, stage, publish |
| 32 | System inspection | scoped facts |
| 33 | Interactive cluster monitor | live topology |
| 34 | Capability-guarded control | safe mutation |
| 35 | Atomic patching and hot swap | stage and rollback |
| 36 | Package audit | find obsolete/risky artifacts |
| 37–39 | Shield, incident response, supply chain | defend and prove |
| 40–42 | Intra/inter-cluster balancing | place work and arbitrate |
| 43–46 | RAS, real-time, quotas, time | predictable operations |
| 47 | Developer ecosystem | debug the platform |
| 48 | Network firewall | filter by authority |
| 49 | In-memory key/value cache | fast bounded state |
| 50 | Remote storage/NAS | enterprise persistence |
| 51 | Confidential computing | attest before trust |
| 52 | Time-travel replay | reproduce failure |
| 53 | Semantic memory | retrieve authorized context |
| 54 | Self-healing daemons | recover clean state |
| 55 | POSIX/Linux compatibility | reduce migration pain |
| 56 | Dynamic cluster mesh | edge to cloud |
| 57 | Declarative infrastructure | signed atomic config |
| 58 | Cluster lifecycle administration | create, join, fence, recover |
| 59 | Project-wide test program | prove every feature |

The TODO also contains detailed checklists under the core filesystem, interactive shell, VM, cluster, and test-program sections. Those checklists are the acceptance criteria; this table is only the memory map.

