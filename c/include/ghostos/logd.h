#ifndef GHOSTOS_LOGD_H
#define GHOSTOS_LOGD_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Subscribe: 0 update existing, 1 insert, 2 capacity.
 * Unsubscribe: 0 found, 1 unknown. Levels match the observability discriminants. */
typedef struct {
    uint32_t terminal;
    uint8_t minimum_level;
    bool occupied;
} ghostos_log_subscription;
int ghostos_log_subscribe(const ghostos_log_subscription *slots, size_t count,
    uint32_t terminal, uint8_t minimum_level, size_t *index);
int ghostos_log_unsubscribe(const ghostos_log_subscription *slots, size_t count,
    uint32_t terminal, size_t *index);
bool ghostos_log_deliver(uint8_t level, uint8_t minimum_level);
bool ghostos_log_operator(uint8_t level, uint8_t kind);
uint64_t ghostos_log_dropped_delta(uint64_t current, uint64_t previous);
size_t ghostos_log_remaining(size_t budget, size_t used);
#endif
