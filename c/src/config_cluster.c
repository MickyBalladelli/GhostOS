#include "ghostos/config_cluster.h"
static bool space(uint8_t byte) { return byte == ' ' || byte == '\t' || byte == '\r'; }
static void trim(const uint8_t **bytes, size_t *length) {
    while (*length && space((*bytes)[0])) { *bytes += 1; *length -= 1; }
    while (*length && space((*bytes)[*length - 1])) *length -= 1;
}
static bool text_is(const uint8_t *bytes, size_t length, const char *text) {
    size_t i;
    for (i = 0; text[i]; ++i) if (i == length || bytes[i] != (uint8_t)text[i]) return false;
    return i == length;
}
static bool either(const uint8_t *bytes, size_t length, const char *left, const char *right) {
    return text_is(bytes, length, left) || text_is(bytes, length, right);
}
static bool bracket(const uint8_t *line, size_t length, bool array, const char *name) {
    size_t name_length = 0, i;
    while (name[name_length]) name_length += 1;
    if (array) {
        if (length != name_length + 4 || line[0] != '[' || line[1] != '[' || line[length - 1] != ']' || line[length - 2] != ']') return false;
        line += 2;
    } else {
        if (length != name_length + 2 || line[0] != '[' || line[length - 1] != ']') return false;
        line += 1;
    }
    for (i = 0; i < name_length; ++i) if (line[i] != (uint8_t)name[i]) return false;
    return true;
}
static void word(const uint8_t *bytes, size_t length, const uint8_t **text, size_t *text_length) {
    if (length >= 2 && ((bytes[0] == '"' && bytes[length - 1] == '"') || (bytes[0] == '\'' && bytes[length - 1] == '\''))) {
        *text = bytes + 1;
        *text_length = length - 2;
    } else {
        *text = bytes;
        *text_length = length;
    }
}
static int quoted(const uint8_t *bytes, size_t length, size_t maximum, const uint8_t **text, size_t *text_length) {
    size_t i;
    if (length < 2) return 11;
    if (bytes[0] == '"' && bytes[length - 1] == '"') {
        for (i = 1; i + 1 < length; ++i) if (bytes[i] == '\\') return 11;
    } else if (!(bytes[0] == '\'' && bytes[length - 1] == '\'')) return 11;
    *text = bytes + 1;
    *text_length = length - 2;
    if (!*text_length || *text_length > maximum) return 11;
    for (i = 0; i < *text_length; ++i) if (!(*text)[i]) return 11;
    return 0;
}
static int scale_add(uint64_t *high, uint64_t *low, uint64_t base, uint64_t digit) {
    uint64_t low_lo = (*low & 0xffffffffu) * base;
    uint64_t low_hi = (*low >> 32) * base + (low_lo >> 32);
    uint64_t carry = low_hi >> 32;
    uint64_t high_lo, high_hi;
    low_lo = (low_lo & 0xffffffffu) | (low_hi << 32);
    high_lo = (*high & 0xffffffffu) * base + carry;
    high_hi = (*high >> 32) * base + (high_lo >> 32);
    if (high_hi > 0xffffffffu) return 5;
    high_lo = (high_lo & 0xffffffffu) | (high_hi << 32);
    if (low_lo > UINT64_MAX - digit) {
        if (high_lo == UINT64_MAX) return 5;
        high_lo += 1;
    }
    *low = low_lo + digit;
    *high = high_lo;
    return 0;
}
static int integer(const uint8_t *bytes, size_t length, uint64_t *high, uint64_t *low) {
    size_t i = 0;
    uint64_t base = 10;
    *high = 0;
    *low = 0;
    if (length >= 2 && bytes[0] == '0' && bytes[1] == 'x') { base = 16; i = 2; if (length == 2) return 5; }
    if (i == length) return 5;
    for (; i < length; ++i) {
        uint64_t digit;
        if (bytes[i] >= '0' && bytes[i] <= '9') digit = (uint64_t)(bytes[i] - '0');
        else if (base == 16 && bytes[i] >= 'a' && bytes[i] <= 'f') digit = (uint64_t)(bytes[i] - 'a' + 10);
        else if (base == 16 && bytes[i] >= 'A' && bytes[i] <= 'F') digit = (uint64_t)(bytes[i] - 'A' + 10);
        else return 5;
        if (digit >= base) return 5;
        if (scale_add(high, low, base, digit)) return 5;
    }
    return 0;
}
static int narrow(const uint8_t *bytes, size_t length, uint64_t limit, uint64_t *value) {
    uint64_t high = 0, low = 0;
    int status = integer(bytes, length, &high, &low);
    if (status) return status;
    if (high || low > limit) return 5;
    *value = low;
    return 0;
}
static int boolean(const uint8_t *bytes, size_t length, bool *value) {
    if (text_is(bytes, length, "true")) { *value = true; return 0; }
    if (text_is(bytes, length, "false")) { *value = false; return 0; }
    return 8;
}
static int choice(const uint8_t *bytes, size_t length, const char *const *words, size_t count, uint8_t *value) {
    const uint8_t *text;
    size_t text_length, i;
    word(bytes, length, &text, &text_length);
    for (i = 0; i < count; ++i) if (text_is(text, text_length, words[i])) { *value = (uint8_t)i; return 0; }
    return 2;
}
static int copy_text(uint8_t *destination, size_t maximum, const uint8_t *text, size_t length, uint8_t *stored) {
    size_t i;
    if (!length || length > maximum) return 11;
    for (i = 0; i < length; ++i) destination[i] = text[i];
    *stored = (uint8_t)length;
    return 0;
}
static bool same_name(const uint8_t *left, uint8_t left_length, const uint8_t *right, uint8_t right_length) {
    uint8_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static void defaults(ghostos_config_cluster *cluster) {
    static const uint8_t name[] = {'g', 'h', 'o', 's', 't', 'o', 's'};
    size_t i;
    cluster->id_high = 0;
    cluster->id_low = 1;
    for (i = 0; i < 7; ++i) cluster->name[i] = name[i];
    cluster->name_length = 7;
    cluster->discovery = 4;
    cluster->membership = 0;
    cluster->admission = 1;
    cluster->voting_members = 1;
    cluster->required_votes = 1;
    cluster->read_only_without_quorum = true;
    cluster->heartbeat_period_us = 1000000;
    cluster->missed_heartbeat_limit = 5;
    cluster->require_signed_commits = true;
    cluster->require_mutual_identity = true;
    cluster->require_attestation = false;
    cluster->encrypt_control_plane = true;
    cluster->encrypt_data_plane = true;
    cluster->trust_root_high = 0;
    cluster->trust_root_low = 1;
    cluster->cpu_limit = UINT64_MAX;
    cluster->memory_limit_bytes = UINT64_MAX;
    cluster->cxl_limit_bytes = UINT64_MAX;
    cluster->storage_limit_bytes = UINT64_MAX;
    cluster->network_limit_mbps = UINT64_MAX;
    cluster->federation_enabled = false;
    cluster->allow_remote_workloads = false;
    cluster->federation_require_attestation = true;
    cluster->lease_ttl_us = 3600000000ull;
    cluster->max_leases = 0;
}
static int validate(const ghostos_config_cluster *cluster, const ghostos_config_transport *transports, size_t transport_count,
    const ghostos_config_override *overrides, size_t override_count) {
    size_t i;
    if ((cluster->id_high == 0 && cluster->id_low == 0) || !cluster->name_length) return 2;
    if (!cluster->voting_members || !cluster->required_votes || cluster->required_votes > cluster->voting_members) return 2;
    if (!cluster->heartbeat_period_us || !cluster->missed_heartbeat_limit) return 2;
    if (cluster->require_attestation && cluster->trust_root_high == 0 && cluster->trust_root_low == 0) return 2;
    if (cluster->federation_enabled && (!cluster->lease_ttl_us || !cluster->max_leases)) return 2;
    for (i = 0; i < transport_count; ++i) {
        size_t previous;
        if (!transports[i].name_length || transports[i].mtu < 576 || transports[i].mtu > 65535) return 2;
        if (transports[i].enabled && transports[i].kind != 0 && !transports[i].endpoint_length) return 2;
        for (previous = 0; previous < i; ++previous)
            if (same_name(transports[previous].name, transports[previous].name_length, transports[i].name, transports[i].name_length)) return 10;
    }
    for (i = 0; i < override_count; ++i) {
        size_t previous;
        if (!overrides[i].node || (overrides[i].has_heartbeat && !overrides[i].heartbeat_period_us) ||
            (overrides[i].has_missed && !overrides[i].missed_heartbeat_limit)) return 2;
        for (previous = 0; previous < i; ++previous) if (overrides[previous].node == overrides[i].node) return 10;
    }
    return 0;
}
uint64_t ghostos_config_cluster_heartbeat(const ghostos_config_cluster *cluster,
    const ghostos_config_override *overrides, size_t override_count, uint32_t node) {
    size_t i;
    for (i = 0; i < override_count; ++i)
        if (overrides[i].node == node && overrides[i].has_heartbeat) return overrides[i].heartbeat_period_us;
    return cluster->heartbeat_period_us;
}
int ghostos_config_parse_cluster(const uint8_t *source, size_t length, uint64_t *revision,
    ghostos_config_cluster *cluster, ghostos_config_transport *transports, size_t transport_capacity,
    size_t *transport_count, ghostos_config_override *overrides, size_t override_capacity, size_t *override_count) {
    static const char *discovery_words[] = {"disabled", "static", "mesh", "mdns", "hybrid"};
    static const char *membership_words[] = {"static", "automatic"};
    static const char *admission_words[] = {"open", "invitation", "attested"};
    static const char *transport_words[] = {"loopback", "ethernet", "cxl", "wireless", "tunnel"};
    bool has_schema = false, has_revision = false, explicit_transport = false;
    bool cluster_active = false, quorum_active = false, security_active = false, resource_active = false, federation_active = false;
    bool transport_active = false, override_active = false;
    bool has_id = false, has_name = false, has_discovery = false, has_membership = false, has_admission = false;
    bool has_heartbeat = false, has_missed = false, has_votes = false, has_required = false, has_readonly = false;
    bool has_signed = false, has_mutual = false, has_attest = false, has_control = false, has_data = false, has_trust = false;
    bool has_cpu = false, has_memory = false, has_cxl = false, has_storage = false, has_network = false;
    bool has_fed = false, has_remote = false, has_fed_attest = false, has_lease = false, has_max = false;
    bool has_transport_name = false, has_kind = false, has_endpoint = false, has_transport_enabled = false, has_priority = false, has_mtu = false;
    bool has_node = false, has_override_discovery = false, has_override_admission = false, has_override_heartbeat = false;
    bool has_override_missed = false, has_override_transport = false;
    uint8_t section = 0, discovery = 0, membership = 0, admission = 0, kind = 0, override_discovery = 0, override_admission = 0, override_transport = 0;
    uint8_t name[GHOSTOS_CONFIG_CLUSTER_NAME], transport_name[GHOSTOS_CONFIG_TRANSPORT_NAME], endpoint[GHOSTOS_CONFIG_ENDPOINT];
    uint8_t name_length = 0, transport_name_length = 0, endpoint_length = 0;
    uint16_t schema = 0, votes = 0, required = 0, missed = 0, priority = 0, override_missed = 0;
    uint32_t node = 0, max_leases = 0, mtu = 0;
    uint64_t id_high = 0, id_low = 0, heartbeat = 0, trust_high = 0, trust_low = 0, lease = 0, override_heartbeat = 0;
    uint64_t cpu = 0, memory = 0, cxl = 0, storage = 0, network = 0;
    bool readonly = false, signed_commits = false, mutual = false, attest = false, control = false, data = false;
    bool transport_enabled = false, fed = false, remote = false, fed_attest = false;
    size_t cursor = 0;
    defaults(cluster);
    *revision = 0;
    *transport_count = 0;
    *override_count = 0;
    while (cursor < length) {
        const uint8_t *line = source + cursor;
        size_t line_length = 0, hash;
        int status = 0;
        while (cursor + line_length < length && source[cursor + line_length] != '\n') line_length += 1;
        cursor += line_length + (cursor + line_length < length ? 1 : 0);
        for (hash = 0; hash < line_length; ++hash) if (line[hash] == '#') { line_length = hash; break; }
        trim(&line, &line_length);
        if (!line_length) continue;
        if (line[0] == '[') {
            if (transport_active) {
                if (!has_transport_name || !has_kind) return 7;
                if (*transport_count == transport_capacity) return 12;
                if (copy_text(transports[*transport_count].name, GHOSTOS_CONFIG_TRANSPORT_NAME, transport_name, transport_name_length, &transports[*transport_count].name_length)) return 11;
                if (has_endpoint) {
                    if (copy_text(transports[*transport_count].endpoint, GHOSTOS_CONFIG_ENDPOINT, endpoint, endpoint_length, &transports[*transport_count].endpoint_length)) return 11;
                } else transports[*transport_count].endpoint_length = 0;
                transports[*transport_count].kind = kind;
                transports[*transport_count].enabled = has_transport_enabled ? transport_enabled : true;
                transports[*transport_count].priority = has_priority ? priority : 100;
                transports[*transport_count].mtu = has_mtu ? mtu : 1500;
                *transport_count += 1;
                explicit_transport = true;
                transport_active = false;
            }
            if (override_active) {
                if (!has_node) return 7;
                if (*override_count == override_capacity) return 12;
                overrides[*override_count].node = node;
                overrides[*override_count].has_discovery = has_override_discovery;
                overrides[*override_count].has_admission = has_override_admission;
                overrides[*override_count].has_heartbeat = has_override_heartbeat;
                overrides[*override_count].has_missed = has_override_missed;
                overrides[*override_count].has_transport = has_override_transport;
                overrides[*override_count].discovery = override_discovery;
                overrides[*override_count].admission = override_admission;
                overrides[*override_count].transport = override_transport;
                overrides[*override_count].heartbeat_period_us = override_heartbeat;
                overrides[*override_count].missed_heartbeat_limit = override_missed;
                *override_count += 1;
                override_active = false;
            }
            cluster_active = quorum_active = security_active = resource_active = federation_active = false;
            if (bracket(line, line_length, false, "system")) section = 1;
            else if (bracket(line, line_length, false, "cluster")) {
                section = 2; cluster_active = true;
                has_id = has_name = has_discovery = has_membership = has_admission = has_heartbeat = has_missed = false;
            } else if (bracket(line, line_length, false, "cluster.quorum")) {
                section = 3; quorum_active = true; has_votes = has_required = has_readonly = false;
            } else if (bracket(line, line_length, false, "cluster.security")) {
                section = 4; security_active = true; has_signed = has_mutual = has_attest = has_control = has_data = has_trust = false;
            } else if (bracket(line, line_length, false, "cluster.resources")) {
                section = 5; resource_active = true; has_cpu = has_memory = has_cxl = has_storage = has_network = false;
            } else if (bracket(line, line_length, false, "cluster.federation")) {
                section = 6; federation_active = true; has_fed = has_remote = has_fed_attest = has_lease = has_max = false;
            } else if (bracket(line, line_length, true, "cluster.transport") || bracket(line, line_length, true, "cluster.transports")) {
                section = 7; transport_active = true;
                has_transport_name = has_kind = has_endpoint = has_transport_enabled = has_priority = has_mtu = false;
            } else if (bracket(line, line_length, true, "cluster.node-override") || bracket(line, line_length, true, "cluster.node_override") ||
                bracket(line, line_length, true, "cluster.node_overrides")) {
                section = 8; override_active = true;
                has_node = has_override_discovery = has_override_admission = has_override_heartbeat = has_override_missed = has_override_transport = false;
            } else return 1;
            continue;
        }
        {
            size_t equals;
            const uint8_t *key, *value, *text;
            size_t key_length, value_length, text_length = 0;
            uint64_t parsed = 0;
            for (equals = 0; equals < line_length && line[equals] != '='; ++equals) {}
            if (equals == line_length) return 2;
            key = line; key_length = equals; trim(&key, &key_length);
            value = line + equals + 1; value_length = line_length - equals - 1; trim(&value, &value_length);
            if (!key_length) return 2;
            if (section == 1) {
                if (text_is(key, key_length, "schema")) {
                    if (has_schema) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    schema = (uint16_t)parsed; has_schema = true;
                } else if (text_is(key, key_length, "revision")) {
                    if (has_revision) return 4;
                    status = narrow(value, value_length, UINT64_MAX, &parsed);
                    if (status) return status;
                    *revision = parsed; has_revision = true;
                } else return 3;
            } else if (section == 2) {
                if (text_is(key, key_length, "id")) {
                    if (has_id) return 4;
                    status = integer(value, value_length, &id_high, &id_low);
                    if (status) return status;
                    cluster->id_high = id_high; cluster->id_low = id_low; has_id = true;
                } else if (text_is(key, key_length, "name")) {
                    if (has_name) return 4;
                    status = quoted(value, value_length, GHOSTOS_CONFIG_CLUSTER_NAME, &text, &text_length);
                    if (status) return status;
                    status = copy_text(cluster->name, GHOSTOS_CONFIG_CLUSTER_NAME, text, text_length, &cluster->name_length);
                    if (status) return status;
                    name_length = cluster->name_length; has_name = true; (void)name;
                } else if (text_is(key, key_length, "discovery")) {
                    if (has_discovery) return 4;
                    status = choice(value, value_length, discovery_words, 5, &discovery);
                    if (status) return status;
                    cluster->discovery = discovery; has_discovery = true;
                } else if (text_is(key, key_length, "membership")) {
                    if (has_membership) return 4;
                    status = choice(value, value_length, membership_words, 2, &membership);
                    if (status) return status;
                    cluster->membership = membership; has_membership = true;
                } else if (text_is(key, key_length, "admission")) {
                    if (has_admission) return 4;
                    status = choice(value, value_length, admission_words, 3, &admission);
                    if (status) return status;
                    cluster->admission = admission; has_admission = true;
                } else if (either(key, key_length, "heartbeat_period_us", "heartbeat-period-us")) {
                    if (has_heartbeat) return 4;
                    status = narrow(value, value_length, UINT64_MAX, &heartbeat);
                    if (status) return status;
                    cluster->heartbeat_period_us = heartbeat; has_heartbeat = true;
                } else if (either(key, key_length, "missed_heartbeat_limit", "missed-heartbeat-limit")) {
                    if (has_missed) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    cluster->missed_heartbeat_limit = (uint16_t)parsed; missed = (uint16_t)parsed; has_missed = true;
                } else return 3;
            } else if (section == 3) {
                if (either(key, key_length, "voting_members", "voting-members")) {
                    if (has_votes) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    cluster->voting_members = votes = (uint16_t)parsed; has_votes = true;
                } else if (either(key, key_length, "required_votes", "required-votes")) {
                    if (has_required) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    cluster->required_votes = required = (uint16_t)parsed; has_required = true;
                } else if (either(key, key_length, "read_only_without_quorum", "read-only-without-quorum")) {
                    if (has_readonly) return 4;
                    status = boolean(value, value_length, &readonly);
                    if (status) return status;
                    cluster->read_only_without_quorum = readonly; has_readonly = true;
                } else return 3;
            } else if (section == 4) {
                if (either(key, key_length, "require_signed_commits", "require-signed-commits")) {
                    if (has_signed) return 4;
                    status = boolean(value, value_length, &signed_commits);
                    if (status) return status;
                    cluster->require_signed_commits = signed_commits; has_signed = true;
                } else if (either(key, key_length, "require_mutual_identity", "require-mutual-identity")) {
                    if (has_mutual) return 4;
                    status = boolean(value, value_length, &mutual);
                    if (status) return status;
                    cluster->require_mutual_identity = mutual; has_mutual = true;
                } else if (either(key, key_length, "require_attestation", "require-attestation")) {
                    if (has_attest) return 4;
                    status = boolean(value, value_length, &attest);
                    if (status) return status;
                    cluster->require_attestation = attest; has_attest = true;
                } else if (either(key, key_length, "encrypt_control_plane", "encrypt-control-plane")) {
                    if (has_control) return 4;
                    status = boolean(value, value_length, &control);
                    if (status) return status;
                    cluster->encrypt_control_plane = control; has_control = true;
                } else if (either(key, key_length, "encrypt_data_plane", "encrypt-data-plane")) {
                    if (has_data) return 4;
                    status = boolean(value, value_length, &data);
                    if (status) return status;
                    cluster->encrypt_data_plane = data; has_data = true;
                } else if (either(key, key_length, "trust_root", "trust-root")) {
                    if (has_trust) return 4;
                    status = integer(value, value_length, &trust_high, &trust_low);
                    if (status) return status;
                    cluster->trust_root_high = trust_high; cluster->trust_root_low = trust_low; has_trust = true;
                } else return 3;
            } else if (section == 5) {
                uint64_t *slot = 0;
                bool *seen = 0;
                if (either(key, key_length, "cpu_limit", "cpu-limit")) { slot = &cluster->cpu_limit; seen = &has_cpu; }
                else if (either(key, key_length, "memory_limit_bytes", "memory-limit-bytes")) { slot = &cluster->memory_limit_bytes; seen = &has_memory; }
                else if (either(key, key_length, "cxl_limit_bytes", "cxl-limit-bytes")) { slot = &cluster->cxl_limit_bytes; seen = &has_cxl; }
                else if (either(key, key_length, "storage_limit_bytes", "storage-limit-bytes")) { slot = &cluster->storage_limit_bytes; seen = &has_storage; }
                else if (either(key, key_length, "network_limit_mbps", "network-limit-mbps")) { slot = &cluster->network_limit_mbps; seen = &has_network; }
                else return 3;
                if (*seen) return 4;
                status = narrow(value, value_length, UINT64_MAX, &parsed);
                if (status) return status;
                *slot = parsed; *seen = true;
                cpu = memory = cxl = storage = network = parsed;
            } else if (section == 6) {
                if (text_is(key, key_length, "enabled")) {
                    if (has_fed) return 4;
                    status = boolean(value, value_length, &fed);
                    if (status) return status;
                    cluster->federation_enabled = fed; has_fed = true;
                } else if (either(key, key_length, "allow_remote_workloads", "allow-remote-workloads")) {
                    if (has_remote) return 4;
                    status = boolean(value, value_length, &remote);
                    if (status) return status;
                    cluster->allow_remote_workloads = remote; has_remote = true;
                } else if (either(key, key_length, "require_attestation", "require-attestation")) {
                    if (has_fed_attest) return 4;
                    status = boolean(value, value_length, &fed_attest);
                    if (status) return status;
                    cluster->federation_require_attestation = fed_attest; has_fed_attest = true;
                } else if (either(key, key_length, "lease_ttl_us", "lease-ttl-us")) {
                    if (has_lease) return 4;
                    status = narrow(value, value_length, UINT64_MAX, &lease);
                    if (status) return status;
                    cluster->lease_ttl_us = lease; has_lease = true;
                } else if (either(key, key_length, "max_leases", "max-leases")) {
                    if (has_max) return 4;
                    status = narrow(value, value_length, UINT32_MAX, &parsed);
                    if (status) return status;
                    cluster->max_leases = max_leases = (uint32_t)parsed; has_max = true;
                } else return 3;
            } else if (section == 7) {
                if (text_is(key, key_length, "name")) {
                    if (has_transport_name) return 4;
                    status = quoted(value, value_length, GHOSTOS_CONFIG_TRANSPORT_NAME, &text, &text_length);
                    if (status) return status;
                    { size_t i; for (i = 0; i < text_length; ++i) transport_name[i] = text[i]; }
                    transport_name_length = (uint8_t)text_length; has_transport_name = true;
                } else if (text_is(key, key_length, "kind")) {
                    if (has_kind) return 4;
                    status = choice(value, value_length, transport_words, 5, &kind);
                    if (status) return status;
                    has_kind = true;
                } else if (text_is(key, key_length, "endpoint")) {
                    if (has_endpoint) return 4;
                    status = quoted(value, value_length, GHOSTOS_CONFIG_ENDPOINT, &text, &text_length);
                    if (status) return status;
                    { size_t i; for (i = 0; i < text_length; ++i) endpoint[i] = text[i]; }
                    endpoint_length = (uint8_t)text_length; has_endpoint = true;
                } else if (text_is(key, key_length, "enabled")) {
                    if (has_transport_enabled) return 4;
                    status = boolean(value, value_length, &transport_enabled);
                    if (status) return status;
                    has_transport_enabled = true;
                } else if (text_is(key, key_length, "priority")) {
                    if (has_priority) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    priority = (uint16_t)parsed; has_priority = true;
                } else if (text_is(key, key_length, "mtu")) {
                    if (has_mtu) return 4;
                    status = narrow(value, value_length, UINT32_MAX, &parsed);
                    if (status) return status;
                    mtu = (uint32_t)parsed; has_mtu = true;
                } else return 3;
            } else if (section == 8) {
                if (text_is(key, key_length, "node")) {
                    if (has_node) return 4;
                    status = narrow(value, value_length, UINT32_MAX, &parsed);
                    if (status) return status;
                    node = (uint32_t)parsed; has_node = true;
                } else if (text_is(key, key_length, "discovery")) {
                    if (has_override_discovery) return 4;
                    status = choice(value, value_length, discovery_words, 5, &override_discovery);
                    if (status) return status;
                    has_override_discovery = true;
                } else if (text_is(key, key_length, "admission")) {
                    if (has_override_admission) return 4;
                    status = choice(value, value_length, admission_words, 3, &override_admission);
                    if (status) return status;
                    has_override_admission = true;
                } else if (either(key, key_length, "heartbeat_period_us", "heartbeat-period-us")) {
                    if (has_override_heartbeat) return 4;
                    status = narrow(value, value_length, UINT64_MAX, &override_heartbeat);
                    if (status) return status;
                    has_override_heartbeat = true;
                } else if (either(key, key_length, "missed_heartbeat_limit", "missed-heartbeat-limit")) {
                    if (has_override_missed) return 4;
                    status = narrow(value, value_length, UINT16_MAX, &parsed);
                    if (status) return status;
                    override_missed = (uint16_t)parsed; has_override_missed = true;
                } else if (text_is(key, key_length, "transport")) {
                    if (has_override_transport) return 4;
                    status = choice(value, value_length, transport_words, 5, &override_transport);
                    if (status) return status;
                    has_override_transport = true;
                } else return 3;
            } else return 3;
            (void)status;
        }
    }
    if (transport_active) {
        if (!has_transport_name || !has_kind) return 7;
        if (*transport_count == transport_capacity) return 12;
        if (copy_text(transports[*transport_count].name, GHOSTOS_CONFIG_TRANSPORT_NAME, transport_name, transport_name_length, &transports[*transport_count].name_length)) return 11;
        if (has_endpoint) {
            if (copy_text(transports[*transport_count].endpoint, GHOSTOS_CONFIG_ENDPOINT, endpoint, endpoint_length, &transports[*transport_count].endpoint_length)) return 11;
        } else transports[*transport_count].endpoint_length = 0;
        transports[*transport_count].kind = kind;
        transports[*transport_count].enabled = has_transport_enabled ? transport_enabled : true;
        transports[*transport_count].priority = has_priority ? priority : 100;
        transports[*transport_count].mtu = has_mtu ? mtu : 1500;
        *transport_count += 1;
        explicit_transport = true;
    }
    if (override_active) {
        if (!has_node) return 7;
        if (*override_count == override_capacity) return 12;
        overrides[*override_count].node = node;
        overrides[*override_count].has_discovery = has_override_discovery;
        overrides[*override_count].has_admission = has_override_admission;
        overrides[*override_count].has_heartbeat = has_override_heartbeat;
        overrides[*override_count].has_missed = has_override_missed;
        overrides[*override_count].has_transport = has_override_transport;
        overrides[*override_count].discovery = override_discovery;
        overrides[*override_count].admission = override_admission;
        overrides[*override_count].transport = override_transport;
        overrides[*override_count].heartbeat_period_us = override_heartbeat;
        overrides[*override_count].missed_heartbeat_limit = override_missed;
        *override_count += 1;
    }
    if (!explicit_transport) {
        static const uint8_t loopback[] = {'l', 'o', 'o', 'p', 'b', 'a', 'c', 'k'};
        if (*transport_count == transport_capacity) return 12;
        if (copy_text(transports[*transport_count].name, GHOSTOS_CONFIG_TRANSPORT_NAME, loopback, 8, &transports[*transport_count].name_length)) return 11;
        transports[*transport_count].endpoint_length = 0;
        transports[*transport_count].kind = 0;
        transports[*transport_count].enabled = true;
        transports[*transport_count].priority = 0;
        transports[*transport_count].mtu = 65535;
        *transport_count += 1;
    }
    if (!has_schema) return 7;
    if (schema != 1) return 6;
    if (!has_revision || !*revision) return !*revision && has_revision ? 2 : 7;
    (void)name_length; (void)votes; (void)required; (void)missed; (void)max_leases;
    (void)cpu; (void)memory; (void)cxl; (void)storage; (void)network;
    (void)cluster_active; (void)quorum_active; (void)security_active; (void)resource_active; (void)federation_active;
    return validate(cluster, transports, *transport_count, overrides, *override_count);
}
