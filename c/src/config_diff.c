#include "ghostos/config_diff.h"
static bool bytes_same(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static bool name_same(const uint8_t *left, uint8_t left_length, const uint8_t *right, uint8_t right_length) {
    return bytes_same(left, left_length, right, right_length);
}
static bool transport_same(const ghostos_config_transport *left, const ghostos_config_transport *right) {
    return name_same(left->name, left->name_length, right->name, right->name_length) &&
        name_same(left->endpoint, left->endpoint_length, right->endpoint, right->endpoint_length) &&
        left->kind == right->kind && left->enabled == right->enabled && left->priority == right->priority && left->mtu == right->mtu;
}
static bool transports_same(const ghostos_config_transport *left, size_t left_count, const ghostos_config_transport *right, size_t right_count) {
    size_t i;
    if (left_count != right_count) return false;
    for (i = 0; i < left_count; ++i) if (!transport_same(&left[i], &right[i])) return false;
    return true;
}
static bool override_same(const ghostos_config_override *left, const ghostos_config_override *right) {
    return left->node == right->node && left->has_discovery == right->has_discovery && left->has_admission == right->has_admission &&
        left->has_heartbeat == right->has_heartbeat && left->has_missed == right->has_missed && left->has_transport == right->has_transport &&
        (!left->has_discovery || left->discovery == right->discovery) && (!left->has_admission || left->admission == right->admission) &&
        (!left->has_heartbeat || left->heartbeat_period_us == right->heartbeat_period_us) &&
        (!left->has_missed || left->missed_heartbeat_limit == right->missed_heartbeat_limit) &&
        (!left->has_transport || left->transport == right->transport);
}
static bool overrides_same(const ghostos_config_override *left, size_t left_count, const ghostos_config_override *right, size_t right_count) {
    size_t i;
    if (left_count != right_count) return false;
    for (i = 0; i < left_count; ++i) if (!override_same(&left[i], &right[i])) return false;
    return true;
}
static bool services_same(const ghostos_config_view *left, const ghostos_config_view *right) {
    size_t i;
    if (left->service_count != right->service_count) return false;
    for (i = 0; i < left->service_count; ++i)
        if (!bytes_same(left->service_names[i], left->service_lengths[i], right->service_names[i], right->service_lengths[i])) return false;
    return true;
}
static int add_area(uint8_t *areas, size_t capacity, size_t *count, uint8_t area) {
    if (*count == capacity) return 1;
    areas[*count] = area;
    *count += 1;
    return 0;
}
static int add_node(uint32_t *nodes, size_t capacity, size_t *count, uint32_t node) {
    size_t i;
    for (i = 0; i < *count; ++i) if (nodes[i] == node) return 0;
    if (*count == capacity) return 1;
    nodes[*count] = node;
    *count += 1;
    return 0;
}
static int add_service(ghostos_config_diff_service *services, size_t capacity, size_t *count, const uint8_t *name, size_t length) {
    size_t i, byte;
    if (!length || length > GHOSTOS_CONFIG_DIFF_SERVICE) return 1;
    for (i = 0; i < *count; ++i) if (bytes_same(services[i].bytes, services[i].length, name, length)) return 0;
    if (*count == capacity) return 1;
    for (byte = 0; byte < length; ++byte) services[*count].bytes[byte] = name[byte];
    services[*count].length = (uint8_t)length;
    *count += 1;
    return 0;
}
static int add_override_nodes(uint32_t *nodes, size_t capacity, size_t *count, const ghostos_config_override *overrides, size_t override_count) {
    size_t i;
    for (i = 0; i < override_count; ++i) if (add_node(nodes, capacity, count, overrides[i].node)) return 1;
    return 0;
}
static int add_service_names(ghostos_config_diff_service *services, size_t capacity, size_t *count, const ghostos_config_view *view) {
    size_t i;
    for (i = 0; i < view->service_count; ++i)
        if (add_service(services, capacity, count, view->service_names[i], view->service_lengths[i])) return 1;
    return 0;
}
int ghostos_config_diff(const ghostos_config_view *previous, const ghostos_config_view *next, bool network_matches,
    uint64_t *from_revision, bool *has_from, uint8_t *areas, size_t area_capacity, size_t *area_count,
    uint32_t *nodes, size_t node_capacity, size_t *node_count, ghostos_config_diff_service *services,
    size_t service_capacity, size_t *service_count, bool *cluster_wide) {
    const ghostos_config_cluster *old_cluster = previous ? previous->cluster : 0;
    const ghostos_config_cluster *new_cluster = next->cluster;
    bool cluster_changed = false;
    bool identity_changed = !old_cluster || old_cluster->id_high != new_cluster->id_high || old_cluster->id_low != new_cluster->id_low ||
        !name_same(old_cluster->name, old_cluster->name_length, new_cluster->name, new_cluster->name_length) ||
        !bytes_same(previous->description, previous->description_length, next->description, next->description_length);
    bool discovery_changed = !old_cluster || old_cluster->discovery != new_cluster->discovery;
    bool membership_changed = !old_cluster || old_cluster->membership != new_cluster->membership || old_cluster->admission != new_cluster->admission;
    bool quorum_changed = !old_cluster || old_cluster->voting_members != new_cluster->voting_members ||
        old_cluster->required_votes != new_cluster->required_votes || old_cluster->read_only_without_quorum != new_cluster->read_only_without_quorum ||
        old_cluster->heartbeat_period_us != new_cluster->heartbeat_period_us || old_cluster->missed_heartbeat_limit != new_cluster->missed_heartbeat_limit;
    bool transport_changed = !old_cluster || !transports_same(previous->transports, previous->transport_count, next->transports, next->transport_count);
    bool security_changed = !old_cluster || old_cluster->require_signed_commits != new_cluster->require_signed_commits ||
        old_cluster->require_mutual_identity != new_cluster->require_mutual_identity ||
        old_cluster->require_attestation != new_cluster->require_attestation ||
        old_cluster->encrypt_control_plane != new_cluster->encrypt_control_plane ||
        old_cluster->encrypt_data_plane != new_cluster->encrypt_data_plane ||
        old_cluster->trust_root_high != new_cluster->trust_root_high || old_cluster->trust_root_low != new_cluster->trust_root_low;
    bool resources_changed = !old_cluster || old_cluster->cpu_limit != new_cluster->cpu_limit ||
        old_cluster->memory_limit_bytes != new_cluster->memory_limit_bytes || old_cluster->cxl_limit_bytes != new_cluster->cxl_limit_bytes ||
        old_cluster->storage_limit_bytes != new_cluster->storage_limit_bytes || old_cluster->network_limit_mbps != new_cluster->network_limit_mbps;
    bool federation_changed = !old_cluster || old_cluster->federation_enabled != new_cluster->federation_enabled ||
        old_cluster->allow_remote_workloads != new_cluster->allow_remote_workloads ||
        old_cluster->federation_require_attestation != new_cluster->federation_require_attestation ||
        old_cluster->lease_ttl_us != new_cluster->lease_ttl_us || old_cluster->max_leases != new_cluster->max_leases;
    bool overrides_changed = !old_cluster || !overrides_same(previous->overrides, previous->override_count, next->overrides, next->override_count);
    *area_count = 0;
    *node_count = 0;
    *service_count = 0;
    *cluster_wide = false;
    *has_from = previous != 0;
    *from_revision = previous ? previous->revision : 0;
    if (identity_changed && add_area(areas, area_capacity, area_count, 0)) return 1;
    if (discovery_changed && add_area(areas, area_capacity, area_count, 1)) return 1;
    if (membership_changed && add_area(areas, area_capacity, area_count, 2)) return 1;
    if (quorum_changed && add_area(areas, area_capacity, area_count, 3)) return 1;
    if (transport_changed && add_area(areas, area_capacity, area_count, 4)) return 1;
    if (security_changed && add_area(areas, area_capacity, area_count, 5)) return 1;
    if (resources_changed && add_area(areas, area_capacity, area_count, 6)) return 1;
    if (federation_changed && add_area(areas, area_capacity, area_count, 7)) return 1;
    if (overrides_changed) {
        if (add_area(areas, area_capacity, area_count, 8)) return 1;
        *cluster_wide = true;
    }
    cluster_changed = identity_changed || discovery_changed || membership_changed || quorum_changed || transport_changed ||
        security_changed || resources_changed || federation_changed || overrides_changed;
    if (cluster_changed) {
        *cluster_wide = true;
        if (previous && add_override_nodes(nodes, node_capacity, node_count, previous->overrides, previous->override_count)) return 1;
        if (add_override_nodes(nodes, node_capacity, node_count, next->overrides, next->override_count)) return 1;
    }
    if (!previous || !services_same(previous, next)) {
        if (add_area(areas, area_capacity, area_count, 9)) return 1;
        if (previous && add_service_names(services, service_capacity, service_count, previous)) return 1;
        if (add_service_names(services, service_capacity, service_count, next)) return 1;
    }
    if (!previous || !network_matches) {
        if (add_area(areas, area_capacity, area_count, 10)) return 1;
        *cluster_wide = true;
    }
    return 0;
}
