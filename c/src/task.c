#include "ghostos/task.h"

bool ghostos_task_cpu_id_valid(uint8_t raw) {
    return raw < GHOSTOS_TASK_MAX_CPUS;
}

bool ghostos_task_cpu_mask_contains(uint64_t low, uint64_t high, uint8_t cpu) {
    if (!ghostos_task_cpu_id_valid(cpu)) return false;
    uint64_t word = cpu < 64 ? low : high;
    unsigned bit = cpu < 64 ? cpu : (unsigned)(cpu - 64);
    return (word & (UINT64_C(1) << bit)) != 0;
}

uint64_t ghostos_task_cpu_mask_union(uint64_t left, uint64_t right) {
    return left | right;
}

uint64_t ghostos_task_cpu_mask_difference(uint64_t left, uint64_t right) {
    return left & ~right;
}

bool ghostos_task_cpu_mask_intersects(uint64_t left_low, uint64_t left_high,
    uint64_t right_low, uint64_t right_high) {
    return (left_low & right_low) != 0 || (left_high & right_high) != 0;
}

void ghostos_task_cpu_mask_from_cpu(uint8_t cpu, uint64_t words[2]) {
    if (!words) return;
    words[0] = words[1] = 0;
    if (!ghostos_task_cpu_id_valid(cpu)) return;
    words[cpu < 64 ? 0 : 1] = UINT64_C(1) << (cpu < 64 ? cpu : (unsigned)(cpu - 64));
}

bool ghostos_task_address_space_id_valid(uint32_t raw) {
    return raw != 0;
}

uint32_t ghostos_task_thread_id_from_parts(size_t slot, uint16_t generation) {
    return ((uint32_t)generation << 16) | (uint32_t)slot;
}

bool ghostos_task_thread_id_valid(uint32_t raw) {
    return raw != 0 && (raw >> 16) != 0;
}

size_t ghostos_task_thread_slot(uint32_t raw) {
    return (size_t)(raw & UINT32_C(0xffff));
}

uint16_t ghostos_task_thread_generation(uint32_t raw) {
    return (uint16_t)(raw >> 16);
}
