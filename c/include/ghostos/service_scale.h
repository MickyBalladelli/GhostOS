#ifndef GHOSTOS_SERVICE_SCALE_H
#define GHOSTOS_SERVICE_SCALE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct {
    uint64_t generation;
    uint32_t id;
    uint16_t sessions, in_flight;
    uint8_t state;
    bool occupied;
} ghostos_scale_instance;
/* State: joining=0, ready=1, draining=2, restarting=3.
 * Error: success=0, capacity=1, duplicate=2, not found=3,
 * invalid state=4, stale generation=5, in flight=6, no target=7.
 * Operation: join=0, ready=1, drain=2, restart=3, checked=4, checked ready=5. */
int ghostos_scale_membership(const ghostos_scale_instance *instances, size_t count,
    uint32_t id, uint64_t generation, uint8_t operation, size_t *index);
int ghostos_scale_target(const ghostos_scale_instance *instances, size_t count,
    uint32_t owner, uint32_t *target);
uint64_t ghostos_scale_digest(const uint8_t *bytes, size_t length);
/* Session state: active=0, prepared=1, accepted=2, closed=3. */
int ghostos_scale_session_close(uint8_t state, uint16_t in_flight);
int ghostos_scale_prepare(uint8_t state, uint16_t in_flight, uint64_t generation,
    uint64_t requested_generation, uint64_t sequence, uint64_t requested_sequence);
typedef struct {
    uint64_t source_generation, target_generation, sequence, digest;
    uint32_t source, target;
} ghostos_scale_handoff;
bool ghostos_scale_same_handoff(const ghostos_scale_handoff *stored, const ghostos_scale_handoff *token);
typedef struct {
    uint64_t id, session, effect;
    uint8_t state;
    bool occupied;
} ghostos_scale_request;
typedef struct { uint64_t effect; bool occupied; } ghostos_scale_effect;
/* Request state: in flight=0, failed=1, completed=2.
 * Decision: new=0, retry=1, in flight=2, completed request=3, completed effect=4.
 * Additional errors: conflict=8, invalid id=9. Index names the source record. */
int ghostos_scale_route(const ghostos_scale_request *requests, size_t request_count,
    const ghostos_scale_effect *effects, size_t effect_count, uint64_t request,
    uint64_t session, uint64_t effect, uint8_t *decision, size_t *index);
int ghostos_scale_request_finish(uint32_t owner, uint64_t generation, uint8_t state,
    uint32_t requested_owner, uint64_t requested_generation);
bool ghostos_scale_increment(uint64_t value, uint64_t maximum, uint64_t *next);
uint16_t ghostos_scale_decrement(uint16_t value);
#endif
