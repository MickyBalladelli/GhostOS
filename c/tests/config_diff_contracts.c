#include "ghostos/config_cluster.h"
#include "ghostos/config_diff.h"
#include <assert.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static const char *SAMPLE =
    "[system]\n"
    "schema = 1\n"
    "revision = 4\n"
    "[cluster]\n"
    "id = 0x1234\n"
    "name = \"prod\"\n"
    "discovery = \"hybrid\"\n"
    "membership = \"automatic\"\n"
    "admission = \"attested\"\n"
    "heartbeat_period_us = 500000\n"
    "missed_heartbeat_limit = 3\n"
    "[cluster.quorum]\n"
    "voting_members = 3\n"
    "required_votes = 2\n"
    "[cluster.security]\n"
    "require_attestation = true\n"
    "trust_root = 0x55\n"
    "[cluster.federation]\n"
    "enabled = true\n"
    "allow_remote_workloads = true\n"
    "lease_ttl_us = 60000000\n"
    "max_leases = 8\n"
    "[[cluster.transport]]\n"
    "name = \"lan\"\n"
    "kind = \"ethernet\"\n"
    "endpoint = \"10.0.0.1:7000\"\n"
    "priority = 10\n"
    "[[cluster.node-override]]\n"
    "node = 2\n"
    "heartbeat_period_us = 100000\n"
    "transport = \"ethernet\"\n";
static void fill(const char *source, uint64_t *revision, ghostos_config_cluster *cluster,
    ghostos_config_transport *transports, size_t *transport_count, ghostos_config_override *overrides, size_t *override_count) {
    assert(!ghostos_config_parse_cluster((const uint8_t *)source, text_length(source), revision, cluster, transports, 2,
        transport_count, overrides, 2, override_count));
}
static void quorum_change_reports_node_two(void) {
    const char *next_source =
        "[system]\n"
        "schema = 1\n"
        "revision = 5\n"
        "[cluster]\n"
        "id = 0x1234\n"
        "name = \"prod\"\n"
        "discovery = \"hybrid\"\n"
        "membership = \"automatic\"\n"
        "admission = \"attested\"\n"
        "heartbeat_period_us = 500000\n"
        "missed_heartbeat_limit = 3\n"
        "[cluster.quorum]\n"
        "voting_members = 3\n"
        "required_votes = 3\n"
        "[cluster.security]\n"
        "require_attestation = true\n"
        "trust_root = 0x55\n"
        "[cluster.federation]\n"
        "enabled = true\n"
        "allow_remote_workloads = true\n"
        "lease_ttl_us = 60000000\n"
        "max_leases = 8\n"
        "[[cluster.transport]]\n"
        "name = \"lan\"\n"
        "kind = \"ethernet\"\n"
        "endpoint = \"10.0.0.1:7000\"\n"
        "priority = 10\n"
        "[[cluster.node-override]]\n"
        "node = 2\n"
        "heartbeat_period_us = 100000\n"
        "transport = \"ethernet\"\n";
    ghostos_config_cluster previous_cluster, next_cluster;
    ghostos_config_transport previous_transports[2], next_transports[2];
    ghostos_config_override previous_overrides[2], next_overrides[2];
    ghostos_config_view previous, next;
    ghostos_config_diff_service services[1];
    uint64_t previous_revision = 0, next_revision = 0, from_revision = 0;
    size_t previous_transport_count = 0, next_transport_count = 0, previous_override_count = 0, next_override_count = 0;
    size_t area_count = 0, node_count = 0, service_count = 0;
    uint8_t areas[4];
    uint32_t nodes[4];
    bool has_from = false, cluster_wide = false;
    fill(SAMPLE, &previous_revision, &previous_cluster, previous_transports, &previous_transport_count, previous_overrides, &previous_override_count);
    fill(next_source, &next_revision, &next_cluster, next_transports, &next_transport_count, next_overrides, &next_override_count);
    previous.revision = previous_revision;
    previous.cluster = &previous_cluster;
    previous.description = 0;
    previous.description_length = 0;
    previous.transports = previous_transports;
    previous.transport_count = previous_transport_count;
    previous.overrides = previous_overrides;
    previous.override_count = previous_override_count;
    previous.service_names = 0;
    previous.service_lengths = 0;
    previous.service_count = 0;
    next = previous;
    next.revision = next_revision;
    next.cluster = &next_cluster;
    next.transports = next_transports;
    next.transport_count = next_transport_count;
    next.overrides = next_overrides;
    next.override_count = next_override_count;
    assert(!ghostos_config_diff(&previous, &next, true, &from_revision, &has_from, areas, 4, &area_count, nodes, 4,
        &node_count, services, 1, &service_count, &cluster_wide));
    assert(has_from && from_revision == 4 && next_revision == 5);
    assert(area_count == 1 && areas[0] == 3 && node_count == 1 && nodes[0] == 2 && cluster_wide && !service_count);
    assert(ghostos_config_diff(&previous, &next, true, &from_revision, &has_from, areas, 0, &area_count, nodes, 4,
        &node_count, services, 1, &service_count, &cluster_wide) == 1);
    assert(!ghostos_config_diff(&previous, &previous, true, &from_revision, &has_from, areas, 4, &area_count, nodes, 4,
        &node_count, services, 1, &service_count, &cluster_wide));
    assert(!area_count && !node_count && !cluster_wide);
}
int main(void) {
    quorum_change_reports_node_two();
    return 0;
}
