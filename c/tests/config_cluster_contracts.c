#include "ghostos/config_cluster.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void cluster_sample_keeps_quorum_transport_and_override(void) {
    const char *source =
        "[system]\n"
        "schema = 1\n"
        "revision = 4\n"
        "\n"
        "[cluster]\n"
        "id = 0x1234\n"
        "name = \"prod\"\n"
        "discovery = \"hybrid\"\n"
        "membership = \"automatic\"\n"
        "admission = \"attested\"\n"
        "heartbeat_period_us = 500000\n"
        "missed_heartbeat_limit = 3\n"
        "\n"
        "[cluster.quorum]\n"
        "voting_members = 3\n"
        "required_votes = 2\n"
        "\n"
        "[cluster.security]\n"
        "require_attestation = true\n"
        "trust_root = 0x55\n"
        "\n"
        "[cluster.federation]\n"
        "enabled = true\n"
        "allow_remote_workloads = true\n"
        "lease_ttl_us = 60000000\n"
        "max_leases = 8\n"
        "\n"
        "[[cluster.transport]]\n"
        "name = \"lan\"\n"
        "kind = \"ethernet\"\n"
        "endpoint = \"10.0.0.1:7000\"\n"
        "priority = 10\n"
        "\n"
        "[[cluster.node-override]]\n"
        "node = 2\n"
        "heartbeat_period_us = 100000\n"
        "transport = \"ethernet\"\n";
    const char *over_quorum =
        "[system]\nschema = 1\nrevision = 4\n"
        "[cluster.quorum]\nvoting_members = 3\nrequired_votes = 4\n";
    ghostos_config_cluster cluster;
    ghostos_config_transport transports[2];
    ghostos_config_override overrides[2];
    uint64_t revision = 0;
    size_t transport_count = 0, override_count = 0;
    assert(!ghostos_config_parse_cluster((const uint8_t *)source, text_length(source), &revision, &cluster,
        transports, 2, &transport_count, overrides, 2, &override_count));
    assert(revision == 4 && cluster.id_high == 0 && cluster.id_low == 0x1234);
    assert(cluster.name_length == 4 && !memcmp(cluster.name, "prod", 4));
    assert(cluster.discovery == 4 && cluster.membership == 1 && cluster.admission == 2);
    assert(cluster.voting_members == 3 && cluster.required_votes == 2);
    assert(cluster.require_attestation && cluster.trust_root_low == 0x55);
    assert(cluster.federation_enabled && cluster.max_leases == 8);
    assert(transport_count == 1 && transports[0].kind == 1 && transports[0].priority == 10 && transports[0].mtu == 1500);
    assert(transports[0].name_length == 3 && !memcmp(transports[0].name, "lan", 3));
    assert(override_count == 1 && overrides[0].node == 2 && overrides[0].transport == 1);
    assert(ghostos_config_cluster_heartbeat(&cluster, overrides, override_count, 2) == 100000);
    assert(ghostos_config_cluster_heartbeat(&cluster, overrides, override_count, 1) == 500000);
    assert(ghostos_config_parse_cluster((const uint8_t *)over_quorum, text_length(over_quorum), &revision, &cluster,
        transports, 2, &transport_count, overrides, 2, &override_count) == 2);
}
int main(void) {
    cluster_sample_keeps_quorum_transport_and_override();
    return 0;
}
