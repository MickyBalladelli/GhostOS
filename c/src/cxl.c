#include "ghostos/cxl.h"
static uint64_t sat_add(uint64_t left, uint64_t right) {
    return left > UINT64_MAX - right ? UINT64_MAX : left + right;
}
static uint64_t sat_mul(uint64_t left, uint64_t right) {
    if (!left || !right) return 0;
    return left > UINT64_MAX / right ? UINT64_MAX : left * right;
}
static int find_serial(ghostos_cxl_device *devices, size_t capacity, uint64_t serial, size_t *index) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (devices[i].occupied && devices[i].serial == serial) { *index = i; return 0; }
    return 4;
}
static bool same_channel(const ghostos_cxl_budget *budget, uint32_t node, uint8_t channel) {
    return budget->occupied && budget->node == node && budget->channel == channel && !budget->tenant;
}
static void refill(ghostos_cxl_budget *budget, uint64_t now_us) {
    uint64_t elapsed, produced, added;
    if (now_us < budget->last_refill_us) {
        budget->last_refill_us = now_us;
        budget->remainder = 0;
        return;
    }
    elapsed = now_us - budget->last_refill_us;
    produced = sat_add(sat_mul(elapsed, budget->bytes_per_second), budget->remainder);
    added = produced / 1000000ull;
    budget->remainder = produced % 1000000ull;
    budget->tokens = sat_add(budget->tokens, added);
    if (budget->tokens > budget->burst_bytes) budget->tokens = budget->burst_bytes;
    budget->last_refill_us = now_us;
}
int ghostos_cxl_discover(ghostos_cxl_device *devices, size_t capacity, uint32_t node, uint64_t serial, uint8_t device_type,
    uint64_t register_base, uint32_t register_bytes, uint32_t hdm_offset, uint64_t volatile_capacity,
    uint64_t persistent_capacity, uint8_t decoder_count, size_t *index) {
    size_t slot = 0, i;
    bool found = false;
    uint64_t total;
    if (!register_base || register_bytes < 0x100 || (uint64_t)hdm_offset + 0x30 > register_bytes ||
        volatile_capacity > UINT64_MAX - persistent_capacity)
        return 1;
    total = volatile_capacity + persistent_capacity;
    if (device_type != 3 || !total) return 2;
    if (!decoder_count || decoder_count > 32) return 1;
    if (!find_serial(devices, capacity, serial, &slot)) {
        devices[slot].state = 0;
        devices[slot].decoder_count = decoder_count;
        devices[slot].node = node;
        devices[slot].register_base = register_base;
        devices[slot].register_bytes = register_bytes;
        devices[slot].hdm_offset = hdm_offset;
        devices[slot].volatile_capacity = volatile_capacity;
        devices[slot].persistent_capacity = persistent_capacity;
        *index = slot;
        return 0;
    }
    for (i = 0; i < capacity; ++i) if (!devices[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    devices[slot].occupied = true;
    devices[slot].state = 0;
    devices[slot].decoder_count = decoder_count;
    devices[slot].node = node;
    devices[slot].serial = serial;
    devices[slot].register_base = register_base;
    devices[slot].register_bytes = register_bytes;
    devices[slot].hdm_offset = hdm_offset;
    devices[slot].volatile_capacity = volatile_capacity;
    devices[slot].persistent_capacity = persistent_capacity;
    *index = slot;
    return 0;
}
int ghostos_cxl_begin_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial, uint8_t *state) {
    size_t index = 0;
    int status = find_serial(devices, capacity, serial, &index);
    if (status) return status;
    if (devices[index].state == 1) return 5;
    devices[index].state = 1;
    *state = devices[index].state;
    return 0;
}
int ghostos_cxl_cancel_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial) {
    size_t index = 0;
    int status = find_serial(devices, capacity, serial, &index);
    if (status) return status;
    devices[index].state = 0;
    return 0;
}
int ghostos_cxl_complete_remove(ghostos_cxl_device *devices, size_t capacity, uint64_t serial, uint64_t *removed_serial) {
    size_t index = 0;
    int status = find_serial(devices, capacity, serial, &index);
    if (status) return status;
    if (devices[index].state != 1) return 5;
    *removed_serial = devices[index].serial;
    devices[index].occupied = false;
    return 0;
}
int ghostos_cxl_configure(ghostos_cxl_budget *budgets, size_t capacity, uint32_t node, uint8_t channel, uint64_t burst_bytes,
    uint64_t bytes_per_second) {
    size_t i, slot = 0;
    bool found = false;
    if (!burst_bytes || !bytes_per_second) return 6;
    for (i = 0; i < capacity; ++i) {
        if (same_channel(&budgets[i], node, channel) || !budgets[i].occupied) { slot = i; found = true; break; }
    }
    if (!found) return 3;
    budgets[slot].occupied = true;
    budgets[slot].node = node;
    budgets[slot].channel = channel;
    budgets[slot].tenant = 0;
    budgets[slot].burst_bytes = burst_bytes;
    budgets[slot].bytes_per_second = bytes_per_second;
    budgets[slot].tokens = burst_bytes;
    budgets[slot].last_refill_us = 0;
    budgets[slot].remainder = 0;
    return 0;
}
int ghostos_cxl_admit(ghostos_cxl_budget *budgets, size_t capacity, uint32_t node, uint8_t channel, uint64_t now_us,
    uint64_t bytes, uint8_t *decision, uint64_t *retry_after_us) {
    size_t i;
    ghostos_cxl_budget *budget = 0;
    uint64_t missing, retry;
    if (!bytes) return 6;
    for (i = 0; i < capacity; ++i) if (same_channel(&budgets[i], node, channel)) { budget = &budgets[i]; break; }
    if (!budget) return 4;
    if (bytes > budget->burst_bytes) { *decision = 1; *retry_after_us = UINT64_MAX; return 0; }
    refill(budget, now_us);
    if (budget->tokens >= bytes) {
        budget->tokens -= bytes;
        *decision = 0;
        *retry_after_us = 0;
        return 0;
    }
    missing = bytes - budget->tokens;
    retry = sat_add(sat_mul(missing, 1000000ull), budget->bytes_per_second - 1) / budget->bytes_per_second;
    if (!retry) retry = 1;
    *decision = 1;
    *retry_after_us = retry;
    return 0;
}
