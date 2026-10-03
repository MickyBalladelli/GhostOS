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
#endif
