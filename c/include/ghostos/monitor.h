#ifndef GHOSTOS_MONITOR_H
#define GHOSTOS_MONITOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MONITOR_MAX_THREADS 64u
#define GHOSTOS_MONITOR_PROCESS_ROWS 16u
#define GHOSTOS_MONITOR_TOP_CPU_ROWS 8u
#define GHOSTOS_MONITOR_LOCK_ROWS 8u
#define GHOSTOS_MONITOR_LOCK_CAPACITY 256u

typedef enum { GHOSTOS_MONITOR_PROCESSES, GHOSTOS_MONITOR_TOP_CPU,
    GHOSTOS_MONITOR_DSM, GHOSTOS_MONITOR_MEMORY } ghostos_monitor_view;
typedef enum { GHOSTOS_MONITOR_VACANT, GHOSTOS_MONITOR_READY, GHOSTOS_MONITOR_RUNNING,
    GHOSTOS_MONITOR_BLOCKED, GHOSTOS_MONITOR_SLEEPING } ghostos_monitor_thread_state;
typedef enum { GHOSTOS_MONITOR_COOPERATIVE, GHOSTOS_MONITOR_REALTIME } ghostos_monitor_policy;

typedef struct {
    uint32_t thread_id;
    uint8_t slot;
    uint8_t state;
    uint8_t policy;
    uint64_t switches;
    uint64_t owner;
    uint32_t address_space;
} ghostos_monitor_thread;

typedef struct {
    uint64_t prev_switches;
    uint8_t samples[10];
    size_t sample_idx;
} ghostos_monitor_cpu_history_entry;

typedef struct {
    uint32_t thread_id;
    uint8_t state;
    uint8_t policy;
    uint64_t switches;
    uint32_t address_space;
} ghostos_monitor_process_info;

typedef struct {
    uint32_t thread_id;
    uint64_t switches;
    uint64_t owner;
    uint32_t address_space;
    uint8_t state;
    uint8_t policy;
    uint8_t util_percent;
} ghostos_monitor_cpu_usage;

typedef struct {
    uint64_t resource_id;
    uint32_t owner_node;
    bool granted;
    bool occupied;
} ghostos_monitor_lock;

typedef struct {
    bool has_resource;
    uint64_t resource_id;
    uint8_t granted;
    uint8_t queued;
    uint32_t owner_node;
} ghostos_monitor_lock_contention;

typedef struct { uint64_t remote_faults, local_faults; uint16_t latency_us; } ghostos_monitor_dsm_page_stats;
typedef void (*ghostos_monitor_write_fn)(void *context, const char *bytes, size_t length);

typedef struct {
    ghostos_monitor_view view;
    ghostos_monitor_cpu_history_entry cpu_history[GHOSTOS_MONITOR_MAX_THREADS];
    uint64_t refresh_counter;
} ghostos_monitor_state;

void ghostos_monitor_init(ghostos_monitor_state *monitor);
void ghostos_monitor_switch_view(ghostos_monitor_state *monitor);
ghostos_monitor_view ghostos_monitor_current_view(const ghostos_monitor_state *monitor);
void ghostos_monitor_update(ghostos_monitor_state *monitor);
size_t ghostos_monitor_get_processes(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_process_info result[GHOSTOS_MONITOR_PROCESS_ROWS]);
size_t ghostos_monitor_get_top_cpu(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_cpu_history_entry history[GHOSTOS_MONITOR_MAX_THREADS],
    ghostos_monitor_cpu_usage result[GHOSTOS_MONITOR_TOP_CPU_ROWS]);
void ghostos_monitor_get_lock_contentions(const ghostos_monitor_lock *locks, size_t lock_count,
    ghostos_monitor_lock_contention result[GHOSTOS_MONITOR_LOCK_ROWS]);
void ghostos_monitor_get_kernel_lock_contentions(
    ghostos_monitor_lock_contention result[GHOSTOS_MONITOR_LOCK_ROWS]);
void ghostos_monitor_get_dsm_stats(ghostos_monitor_dsm_page_stats result[GHOSTOS_MONITOR_LOCK_ROWS]);
void ghostos_monitor_render_processes(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_write_fn write, void *context);
void ghostos_monitor_render_top_cpu(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_cpu_history_entry history[GHOSTOS_MONITOR_MAX_THREADS],
    ghostos_monitor_write_fn write, void *context);
void ghostos_monitor_render_dsm(const ghostos_monitor_lock *locks, size_t lock_count,
    ghostos_monitor_write_fn write, void *context);
void ghostos_monitor_render_kernel_dsm(ghostos_monitor_write_fn write, void *context);
void ghostos_monitor_render_memory(uint64_t available_bytes, uint64_t used_bytes,
    ghostos_monitor_write_fn write, void *context);

#endif
