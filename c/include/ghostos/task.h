#ifndef GHOSTOS_TASK_H
#define GHOSTOS_TASK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_TASK_MAX_CPUS 128u

bool ghostos_task_cpu_id_valid(uint8_t raw);
bool ghostos_task_cpu_mask_contains(uint64_t low, uint64_t high, uint8_t cpu);
uint64_t ghostos_task_cpu_mask_union(uint64_t left, uint64_t right);
uint64_t ghostos_task_cpu_mask_difference(uint64_t left, uint64_t right);
bool ghostos_task_cpu_mask_intersects(uint64_t left_low, uint64_t left_high,
    uint64_t right_low, uint64_t right_high);
void ghostos_task_cpu_mask_from_cpu(uint8_t cpu, uint64_t words[2]);
bool ghostos_task_address_space_id_valid(uint32_t raw);
uint32_t ghostos_task_thread_id_from_parts(size_t slot, uint16_t generation);
bool ghostos_task_thread_id_valid(uint32_t raw);
size_t ghostos_task_thread_slot(uint32_t raw);
uint16_t ghostos_task_thread_generation(uint32_t raw);

#endif
