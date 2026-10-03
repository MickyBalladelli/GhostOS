#ifndef GHOSTOS_WASM_SCRIPT_H
#define GHOSTOS_WASM_SCRIPT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Module: 0 accepted, 1 empty entry, 2 module too large.
 * Grants: 0 accepted, 1 capacity, 2 invalid, 3 duplicate.
 * Invoke: 0 allow, 1 invalid operation, 2 access denied. */
bool ghostos_wasm_limits(uint64_t fuel, size_t max_module_bytes, size_t max_memory_bytes,
    size_t max_table_elements, size_t max_instances, size_t max_memories, size_t max_tables);
int ghostos_wasm_module(size_t wasm_len, size_t max_module_bytes, size_t entry_len);
bool ghostos_wasm_allows(uint64_t operations, uint8_t operation);
int ghostos_wasm_grants(const uint64_t *handles, const uint64_t *operations, size_t count, size_t capacity);
bool ghostos_wasm_find(const uint64_t *handles, const uint64_t *operations, const bool *occupied,
    size_t count, uint64_t handle, uint64_t *found_operations);
int ghostos_wasm_check(bool operation_ok, bool allows);
int ghostos_wasm_invoke(bool operation_ok, bool found, bool allows);
uint64_t ghostos_wasm_fuel_consumed(uint64_t fuel, uint64_t remaining);
#endif
