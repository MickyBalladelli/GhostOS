#ifndef GHOSTOS_PROCESS_H
#define GHOSTOS_PROCESS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PROCESS_CAPACITY 64u

typedef struct {
    uint64_t process_id;
    uint32_t thread_id;
    uint32_t address_space;
    uint64_t authority;
    bool mapping_present;
    uint64_t mapping_base, mapping_size;
    uint64_t limit_memory, limit_cpu_time, limit_deadline, limit_cancel_grace;
    uint64_t usage_memory, usage_cpu_time;
    bool exit_present;
    int32_t exit_status;
    uint8_t exit_kind, crash_kind;
    bool cancel_present;
    uint64_t cancel_time;
} ghostos_process_slot;

typedef struct {
    size_t capacity;
    uint32_t generations[GHOSTOS_PROCESS_CAPACITY];
    ghostos_process_slot slots[GHOSTOS_PROCESS_CAPACITY];
} ghostos_process_table;

typedef enum {
    GHOSTOS_PROCESS_OK = 0,
    GHOSTOS_PROCESS_FULL = 1,
    GHOSTOS_PROCESS_NOT_FOUND = 2,
    GHOSTOS_PROCESS_INVALID_TRANSITION = 3
} ghostos_process_error;

void ghostos_process_table_init(ghostos_process_table *table, size_t capacity);
size_t ghostos_process_free_slot(const ghostos_process_table *table);
ghostos_process_error ghostos_process_new_identity(ghostos_process_table *table, size_t slot,
    uint64_t *process_id, uint32_t *address_space);
size_t ghostos_process_find(const ghostos_process_table *table, uint64_t process_id);
bool ghostos_process_get(const ghostos_process_table *table, size_t slot, ghostos_process_slot *out);
ghostos_process_error ghostos_process_publish(ghostos_process_table *table, size_t slot,
    const ghostos_process_slot *record);
ghostos_process_error ghostos_process_exec(ghostos_process_table *table, size_t slot,
    uint64_t mapping_base, uint64_t mapping_size, uint64_t usage_memory);
ghostos_process_error ghostos_process_cancel(ghostos_process_table *table, size_t slot,
    uint64_t now_us);
ghostos_process_error ghostos_process_finish(ghostos_process_table *table, size_t slot,
    int32_t status, uint8_t exit_kind, uint8_t crash_kind);
void ghostos_process_clear(ghostos_process_table *table, size_t slot);

#endif
