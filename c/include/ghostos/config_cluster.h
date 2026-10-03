#ifndef GHOSTOS_CONFIG_CLUSTER_H
#define GHOSTOS_CONFIG_CLUSTER_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Uses the config parser result codes. Discovery: disabled=0, static=1, mesh=2, mdns=3, hybrid=4.
   Membership: static=0, automatic=1. Admission: open=0, invitation=1, attested=2.
   Transport: loopback=0, ethernet=1, cxl=2, wireless=3, tunnel=4. */
#define GHOSTOS_CONFIG_CLUSTER_NAME 64u
#define GHOSTOS_CONFIG_TRANSPORT_NAME 32u
#define GHOSTOS_CONFIG_ENDPOINT 96u
typedef struct {
    uint64_t id_high, id_low;
    uint8_t name[GHOSTOS_CONFIG_CLUSTER_NAME];
    uint8_t name_length;
    uint8_t discovery, membership, admission;
    uint16_t voting_members, required_votes, missed_heartbeat_limit;
    bool read_only_without_quorum;
    uint64_t heartbeat_period_us;
    bool require_signed_commits, require_mutual_identity, require_attestation;
    bool encrypt_control_plane, encrypt_data_plane;
    uint64_t trust_root_high, trust_root_low;
    uint64_t cpu_limit, memory_limit_bytes, cxl_limit_bytes, storage_limit_bytes, network_limit_mbps;
    bool federation_enabled, allow_remote_workloads, federation_require_attestation;
    uint64_t lease_ttl_us;
    uint32_t max_leases;
} ghostos_config_cluster;
typedef struct {
    uint8_t name[GHOSTOS_CONFIG_TRANSPORT_NAME];
    uint8_t endpoint[GHOSTOS_CONFIG_ENDPOINT];
    uint8_t name_length, endpoint_length, kind;
    bool enabled;
    uint16_t priority;
    uint32_t mtu;
} ghostos_config_transport;
typedef struct {
    uint32_t node;
    bool has_discovery, has_admission, has_heartbeat, has_missed, has_transport;
    uint8_t discovery, admission, transport;
    uint64_t heartbeat_period_us;
    uint16_t missed_heartbeat_limit;
} ghostos_config_override;
int ghostos_config_parse_cluster(const uint8_t *source, size_t length, uint64_t *revision,
    ghostos_config_cluster *cluster, ghostos_config_transport *transports, size_t transport_capacity,
    size_t *transport_count, ghostos_config_override *overrides, size_t override_capacity, size_t *override_count);
uint64_t ghostos_config_cluster_heartbeat(const ghostos_config_cluster *cluster,
    const ghostos_config_override *overrides, size_t override_count, uint32_t node);
#endif
