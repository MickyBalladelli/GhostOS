#include "ghostos/wasm_script.h"
bool ghostos_wasm_limits(uint64_t fuel, size_t max_module_bytes, size_t max_memory_bytes,
    size_t max_table_elements, size_t max_instances, size_t max_memories, size_t max_tables) {
    return fuel && max_module_bytes && max_memory_bytes && max_table_elements && max_instances &&
        max_memories && max_tables;
}
int ghostos_wasm_module(size_t wasm_len, size_t max_module_bytes, size_t entry_len) {
    if (!entry_len) return 1;
    if (wasm_len > max_module_bytes) return 2;
    return 0;
}
bool ghostos_wasm_allows(uint64_t operations, uint8_t operation) {
    return operation < 64 && (operations & (UINT64_C(1) << operation)) != 0;
}
int ghostos_wasm_grants(const uint64_t *handles, const uint64_t *operations, size_t count, size_t capacity) {
    size_t i, j;
    if (count > capacity) return 1;
    for (i = 0; i < count; ++i) {
        if (!handles[i] || !operations[i]) return 2;
        for (j = 0; j < i; ++j) if (handles[j] == handles[i]) return 3;
    }
    return 0;
}
bool ghostos_wasm_find(const uint64_t *handles, const uint64_t *operations, const bool *occupied,
    size_t count, uint64_t handle, uint64_t *found_operations) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (occupied[i] && handles[i] == handle) {
            *found_operations = operations[i];
            return true;
        }
    }
    return false;
}
int ghostos_wasm_check(bool operation_ok, bool allows) {
    return operation_ok && allows ? 1 : 0;
}
int ghostos_wasm_invoke(bool operation_ok, bool found, bool allows) {
    if (!operation_ok) return 1;
    if (!found || !allows) return 2;
    return 0;
}
uint64_t ghostos_wasm_fuel_consumed(uint64_t fuel, uint64_t remaining) {
    return remaining > fuel ? 0 : fuel - remaining;
}
