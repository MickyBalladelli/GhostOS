#ifndef GHOSTOS_CXL_H
#define GHOSTOS_CXL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid device, 2 unsupported, 3 capacity,
   4 device not found, 5 busy, 6 invalid QoS policy.
   Device state: online=0, draining=1. Decision: allowed=0, throttled=1. */
typedef struct {
    bool occupied;
    uint8_t state, decoder_count;
    uint32_t node;
    uint64_t serial, register_base, volatile_capacity, persistent_capacity;
    uint32_t register_bytes, hdm_offset;
} ghostos_cxl_device;
typedef struct {
    bool occupied;
    uint32_t node;
    uint8_t channel;
    uint64_t tenant, burst_bytes, bytes_per_second, tokens, last_refill_us, remainder;
} ghostos_cxl_budget;
int ghostos_cxl_discover(ghostos_cxl_device *devices, size_t capacity, uint32_t node, uint64_t serial, uint8_t device_type,
    uint64_t register_base, uint32_t register_bytes, uint32_t hdm_offset, uint64_t volatile_capacity,
    uint64_t persistent_capacity, uint8_t decoder_count, size_t *index);
int ghostos_cxl_begin_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial, uint8_t *state);
int ghostos_cxl_cancel_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial);
int ghostos_cxl_complete_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial, uint64_t *removed_serial);
int ghostos_cxl_configure(ghostos_cxl_budget *budgets, size_t capacity, uint32_t node, uint8_t channel, uint64_t burst_bytes,
    uint64_t bytes_per_second);
int ghostos_cxl_admit(ghostos_cxl_budget *budgets, size_t capacity, uint32_t node, uint8_t channel, uint64_t now_us,
    uint64_t bytes, uint8_t *decision, uint64_t *retry_after_us);
#endif
