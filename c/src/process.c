#include "ghostos/process.h"

static void clear_slot(ghostos_process_slot *slot) {
    *slot = (ghostos_process_slot){0};
}

void ghostos_process_table_init(ghostos_process_table *table, size_t capacity) {
    if (!table) return;
    if (capacity > GHOSTOS_PROCESS_CAPACITY) capacity = GHOSTOS_PROCESS_CAPACITY;
    table->capacity = capacity;
    for (size_t index = 0; index < GHOSTOS_PROCESS_CAPACITY; ++index) {
        table->generations[index] = 0;
        clear_slot(&table->slots[index]);
    }
}

size_t ghostos_process_free_slot(const ghostos_process_table *table) {
    if (!table) return 0;
    for (size_t index = 0; index < table->capacity; ++index) {
        if (!table->slots[index].process_id) return index;
    }
    return table->capacity;
}

ghostos_process_error ghostos_process_new_identity(ghostos_process_table *table, size_t slot,
    uint64_t *process_id, uint32_t *address_space) {
    if (!table || slot >= table->capacity || !process_id || !address_space || table->slots[slot].process_id) {
        return GHOSTOS_PROCESS_FULL;
    }
    uint32_t generation = table->generations[slot] + 1;
    if (!generation) generation = 1;
    table->generations[slot] = generation;
    uint64_t raw = ((uint64_t)generation << 32) | (slot + 1);
    if (!raw || !(uint32_t)raw) return GHOSTOS_PROCESS_FULL;
    *process_id = raw;
    *address_space = (uint32_t)raw;
    return GHOSTOS_PROCESS_OK;
}

size_t ghostos_process_find(const ghostos_process_table *table, uint64_t process_id) {
    if (!table || !process_id) return table ? table->capacity : 0;
    for (size_t index = 0; index < table->capacity; ++index) {
        if (table->slots[index].process_id == process_id) return index;
    }
    return table->capacity;
}

bool ghostos_process_get(const ghostos_process_table *table, size_t slot, ghostos_process_slot *out) {
    if (!table || !out || slot >= table->capacity || !table->slots[slot].process_id) return false;
    *out = table->slots[slot];
    return true;
}

ghostos_process_error ghostos_process_publish(ghostos_process_table *table, size_t slot,
    const ghostos_process_slot *record) {
    if (!table || !record || slot >= table->capacity || table->slots[slot].process_id || !record->process_id) {
        return GHOSTOS_PROCESS_INVALID_TRANSITION;
    }
    table->slots[slot] = *record;
    return GHOSTOS_PROCESS_OK;
}

ghostos_process_error ghostos_process_exec(ghostos_process_table *table, size_t slot,
    uint64_t base, uint64_t size, uint64_t usage_memory) {
    if (!table || slot >= table->capacity || !table->slots[slot].process_id) return GHOSTOS_PROCESS_NOT_FOUND;
    ghostos_process_slot *record = &table->slots[slot];
    if (record->exit_present) return GHOSTOS_PROCESS_INVALID_TRANSITION;
    record->mapping_present = true;
    record->mapping_base = base;
    record->mapping_size = size;
    record->usage_memory = usage_memory;
    record->cancel_present = false;
    record->cancel_time = 0;
    return GHOSTOS_PROCESS_OK;
}

ghostos_process_error ghostos_process_cancel(ghostos_process_table *table, size_t slot, uint64_t now_us) {
    if (!table || slot >= table->capacity || !table->slots[slot].process_id) return GHOSTOS_PROCESS_NOT_FOUND;
    ghostos_process_slot *record = &table->slots[slot];
    if (record->exit_present) return GHOSTOS_PROCESS_INVALID_TRANSITION;
    if (!record->cancel_present) {
        record->cancel_present = true;
        record->cancel_time = now_us;
    }
    return GHOSTOS_PROCESS_OK;
}

ghostos_process_error ghostos_process_finish(ghostos_process_table *table, size_t slot,
    int32_t status, uint8_t exit_kind, uint8_t crash_kind) {
    if (!table || slot >= table->capacity || !table->slots[slot].process_id) return GHOSTOS_PROCESS_NOT_FOUND;
    ghostos_process_slot *record = &table->slots[slot];
    if (record->exit_present) return GHOSTOS_PROCESS_INVALID_TRANSITION;
    record->exit_present = true;
    record->exit_status = status;
    record->exit_kind = exit_kind;
    record->crash_kind = crash_kind;
    record->mapping_present = false;
    record->cancel_present = false;
    record->cancel_time = 0;
    return GHOSTOS_PROCESS_OK;
}

void ghostos_process_clear(ghostos_process_table *table, size_t slot) {
    if (table && slot < table->capacity) clear_slot(&table->slots[slot]);
}
