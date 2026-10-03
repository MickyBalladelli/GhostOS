#include "ghostos/heartbeat.h"
#include <assert.h>
#include <string.h>
static void heartbeat_ignores_an_older_sequence_and_then_fails(void) {
    uint8_t wire[GHOSTOS_HEARTBEAT_WIRE];
    ghostos_heartbeat_monitor monitor;
    ghostos_heartbeat_node nodes[1];
    uint32_t node = 0, sequence = 0;
    uint64_t sent = 0, due_sent = 0, silence = 0;
    bool ready = true, failed = true;
    uint8_t state = 0;
    assert(!ghostos_heartbeat_encode(2, 4, 77, wire, sizeof wire));
    assert(!ghostos_heartbeat_decode(wire, sizeof wire, &node, &sequence, &sent));
    assert(node == 2 && sequence == 4 && sent == 77);
    assert(!ghostos_heartbeat_init(&monitor, nodes, 1, 1, 100, 2, 0));
    assert(!ghostos_heartbeat_add(&monitor, nodes, 1, 2, 0));
    assert(!ghostos_heartbeat_due(&monitor, 99, &ready, &node, &sequence, &due_sent));
    assert(!ready);
    assert(!ghostos_heartbeat_due(&monitor, 100, &ready, &node, &sequence, &due_sent));
    assert(ready && node == 1 && sequence == 1 && due_sent == 100);
    assert(!ghostos_heartbeat_observe(nodes, 1, 2, 4, 100));
    assert(!ghostos_heartbeat_observe(nodes, 1, 2, 3, 101));
    assert(!ghostos_heartbeat_state(nodes, 1, 2, &state));
    assert(state == 1 && nodes[0].last_seen_us == 100);
    assert(!ghostos_heartbeat_detect(&monitor, nodes, 1, 301, &failed, &node, &silence));
    assert(failed && node == 2 && silence == 201);
    assert(!ghostos_heartbeat_state(nodes, 1, 2, &state));
    assert(state == 2);
}
int main(void) {
    heartbeat_ignores_an_older_sequence_and_then_fails();
    return 0;
}
