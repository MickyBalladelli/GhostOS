#include "ghostos/heartbeat.h"
static void store_be32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)(value >> 24);
    bytes[1] = (uint8_t)(value >> 16);
    bytes[2] = (uint8_t)(value >> 8);
    bytes[3] = (uint8_t)value;
}
static void store_be64(uint8_t *bytes, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) bytes[i] = (uint8_t)(value >> (8 * (7 - i)));
}
static uint32_t load_be32(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) | ((uint32_t)bytes[2] << 8) | bytes[3];
}
static uint64_t load_be64(const uint8_t *bytes) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value = (value << 8) | bytes[i];
    return value;
}
static int find_node(ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node, size_t *index) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (nodes[i].occupied && nodes[i].node == node) { *index = i; return 0; }
    return 4;
}
int ghostos_heartbeat_encode(uint32_t node, uint32_t sequence, uint64_t sent_at_us, uint8_t *bytes, size_t capacity) {
    if (capacity < GHOSTOS_HEARTBEAT_WIRE) return 3;
    store_be32(bytes, node);
    store_be32(bytes + 4, sequence);
    store_be64(bytes + 8, sent_at_us);
    return 0;
}
int ghostos_heartbeat_decode(const uint8_t *bytes, size_t length, uint32_t *node, uint32_t *sequence, uint64_t *sent_at_us) {
    if (length != GHOSTOS_HEARTBEAT_WIRE) return 6;
    *node = load_be32(bytes);
    if (!*node) return 6;
    *sequence = load_be32(bytes + 4);
    *sent_at_us = load_be64(bytes + 8);
    return 0;
}
int ghostos_heartbeat_init(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint32_t local,
    uint32_t period_us, uint8_t missed_limit, uint64_t now_us) {
    size_t i;
    if (!local || !period_us || period_us > 999 || !missed_limit) return 1;
    monitor->local = local;
    monitor->period_us = period_us;
    monitor->missed_limit = missed_limit;
    monitor->next_sequence = 1;
    monitor->next_send_us = now_us > UINT64_MAX - period_us ? UINT64_MAX : now_us + period_us;
    for (i = 0; i < capacity; ++i) {
        nodes[i].occupied = false;
        nodes[i].state = 0;
        nodes[i].node = 0;
        nodes[i].last_sequence = 0;
        nodes[i].last_seen_us = 0;
    }
    return 0;
}
int ghostos_heartbeat_add(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node,
    uint64_t now_us) {
    size_t index = 0, i;
    if (node == monitor->local) return 2;
    if (!find_node(nodes, capacity, node, &index)) {
        nodes[index].state = 1;
        nodes[index].last_sequence = 0;
        nodes[index].last_seen_us = now_us;
        return 0;
    }
    for (i = 0; i < capacity; ++i) if (!nodes[i].occupied) {
        nodes[i].occupied = true;
        nodes[i].node = node;
        nodes[i].state = 1;
        nodes[i].last_sequence = 0;
        nodes[i].last_seen_us = now_us;
        return 0;
    }
    return 3;
}
int ghostos_heartbeat_due(ghostos_heartbeat_monitor *monitor, uint64_t now_us, bool *ready, uint32_t *node, uint32_t *sequence,
    uint64_t *sent_at_us) {
    if (now_us < monitor->next_send_us) { *ready = false; return 0; }
    *ready = true;
    *node = monitor->local;
    *sequence = monitor->next_sequence;
    *sent_at_us = now_us;
    monitor->next_sequence += 1;
    monitor->next_send_us = now_us > UINT64_MAX - monitor->period_us ? UINT64_MAX : now_us + monitor->period_us;
    return 0;
}
int ghostos_heartbeat_observe(ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node, uint32_t sequence, uint64_t received_at_us) {
    size_t index = 0;
    int status = find_node(nodes, capacity, node, &index);
    int32_t delta;
    if (status) return status;
    if (nodes[index].state == 2) return 5;
    delta = (int32_t)(sequence - nodes[index].last_sequence);
    if (nodes[index].last_sequence && delta <= 0) return 0;
    nodes[index].last_sequence = sequence;
    nodes[index].last_seen_us = received_at_us;
    nodes[index].state = 1;
    return 0;
}
int ghostos_heartbeat_detect(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint64_t now_us,
    bool *failed, uint32_t *node, uint64_t *silence_us) {
    uint64_t timeout = (uint64_t)monitor->period_us * monitor->missed_limit;
    size_t i;
    *failed = false;
    for (i = 0; i < capacity; ++i) {
        uint64_t silence = now_us >= nodes[i].last_seen_us ? now_us - nodes[i].last_seen_us : 0;
        if (!nodes[i].occupied || nodes[i].state != 1 || silence < timeout) continue;
        nodes[i].state = 2;
        *failed = true;
        *node = nodes[i].node;
        *silence_us = silence;
        return 0;
    }
    return 0;
}
int ghostos_heartbeat_state(const ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node, uint8_t *state) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (nodes[i].occupied && nodes[i].node == node) { *state = nodes[i].state; return 0; }
    return 4;
}
