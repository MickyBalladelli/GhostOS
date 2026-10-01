#ifndef GHOSTOS_CPU_TOPOLOGY_H
#define GHOSTOS_CPU_TOPOLOGY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_CPUS 128

typedef struct { uint64_t words[2]; } ghostos_cpu_mask;
typedef enum { GHOSTOS_CPU_ABSENT, GHOSTOS_CPU_REQUESTED, GHOSTOS_CPU_STARTED, GHOSTOS_CPU_FAILED } ghostos_cpu_startup_state;
typedef enum { GHOSTOS_CPU_TOPOLOGY_OK, GHOSTOS_CPU_INVALID, GHOSTOS_CPU_DUPLICATE_HARDWARE_ID,
    GHOSTOS_CPU_CAPACITY, GHOSTOS_CPU_INVALID_TRANSITION } ghostos_cpu_topology_error;
typedef struct { uint8_t id; uint32_t hardware_id; ghostos_cpu_startup_state state; } ghostos_cpu_record;
typedef struct { ghostos_cpu_record records[GHOSTOS_MAX_CPUS]; size_t capacity, count; } ghostos_cpu_topology;
typedef struct { uint8_t cpu; uint16_t interrupt_depth; bool enabled, isolated; ghostos_cpu_mask pending_ipi; } ghostos_per_cpu_interrupt_state;

bool ghostos_cpu_id(uint8_t raw, uint8_t *cpu);
ghostos_cpu_mask ghostos_cpu_mask_empty(void);
ghostos_cpu_mask ghostos_cpu_mask_from_cpu(uint8_t cpu);
ghostos_cpu_mask ghostos_cpu_mask_union(ghostos_cpu_mask a, ghostos_cpu_mask b);
ghostos_cpu_mask ghostos_cpu_mask_difference(ghostos_cpu_mask a, ghostos_cpu_mask b);
bool ghostos_cpu_mask_contains(ghostos_cpu_mask mask, uint8_t cpu);
bool ghostos_cpu_mask_intersects(ghostos_cpu_mask a, ghostos_cpu_mask b);
bool ghostos_cpu_mask_is_empty(ghostos_cpu_mask mask);
void ghostos_cpu_topology_init(ghostos_cpu_topology *topology, size_t capacity);
ghostos_cpu_topology_error ghostos_cpu_topology_add_bootstrap(ghostos_cpu_topology *topology, uint32_t hardware_id, uint8_t *cpu);
ghostos_cpu_topology_error ghostos_cpu_topology_request(ghostos_cpu_topology *topology, uint32_t hardware_id, uint8_t *cpu);
ghostos_cpu_topology_error ghostos_cpu_topology_mark_started(ghostos_cpu_topology *topology, uint8_t cpu);
ghostos_cpu_topology_error ghostos_cpu_topology_mark_failed(ghostos_cpu_topology *topology, uint8_t cpu);
ghostos_cpu_topology_error ghostos_cpu_topology_record(const ghostos_cpu_topology *topology, uint8_t cpu, ghostos_cpu_record *record);
ghostos_cpu_mask ghostos_cpu_topology_online(const ghostos_cpu_topology *topology);
size_t ghostos_cpu_topology_online_count(const ghostos_cpu_topology *topology);
size_t ghostos_cpu_topology_records(const ghostos_cpu_topology *topology, ghostos_cpu_record *records, size_t capacity);
void ghostos_per_cpu_interrupt_init(ghostos_per_cpu_interrupt_state *state, uint8_t cpu);
void ghostos_per_cpu_interrupt_enter(ghostos_per_cpu_interrupt_state *state);
void ghostos_per_cpu_interrupt_exit(ghostos_per_cpu_interrupt_state *state);
void ghostos_per_cpu_queue_ipi(ghostos_per_cpu_interrupt_state *state, uint8_t source);
bool ghostos_per_cpu_take_ipi(ghostos_per_cpu_interrupt_state *state, uint8_t source);

#endif
