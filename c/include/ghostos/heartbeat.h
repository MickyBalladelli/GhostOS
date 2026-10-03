#ifndef GHOSTOS_HEARTBEAT_H
#define GHOSTOS_HEARTBEAT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid range, 2 invalid device, 3 capacity,
   4 device not found, 5 node failed, 6 corrupt packet.
   State: vacant=0, alive=1, failed=2. */
#define GHOSTOS_HEARTBEAT_WIRE 16u
typedef struct {
    uint32_t local, period_us, next_sequence;
    uint8_t missed_limit;
    uint64_t next_send_us;
} ghostos_heartbeat_monitor;
typedef struct {
    bool occupied;
    uint8_t state;
    uint32_t node, last_sequence;
    uint64_t last_seen_us;
} ghostos_heartbeat_node;
int ghostos_heartbeat_encode(uint32_t node, uint32_t sequence, uint64_t sent_at_us, uint8_t *bytes, size_t capacity);
int ghostos_heartbeat_decode(const uint8_t *bytes, size_t length, uint32_t *node, uint32_t *sequence, uint64_t *sent_at_us);
int ghostos_heartbeat_init(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint32_t local,
    uint32_t period_us, uint8_t missed_limit, uint64_t now_us);
int ghostos_heartbeat_add(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node,
    uint64_t now_us);
int ghostos_heartbeat_due(ghostos_heartbeat_monitor *monitor, uint64_t now_us, bool *ready, uint32_t *node, uint32_t *sequence,
    uint64_t *sent_at_us);
int ghostos_heartbeat_observe(ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node, uint32_t sequence, uint64_t received_at_us);
int ghostos_heartbeat_detect(ghostos_heartbeat_monitor *monitor, ghostos_heartbeat_node *nodes, size_t capacity, uint64_t now_us,
    bool *failed, uint32_t *node, uint64_t *silence_us);
int ghostos_heartbeat_state(const ghostos_heartbeat_node *nodes, size_t capacity, uint32_t node, uint8_t *state);
#endif
