#ifndef GHOSTOS_EMBEDDED_SCRIPT_H
#define GHOSTOS_EMBEDDED_SCRIPT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Names: 0 accepted, 1 capacity, 2 duplicate.
 * Request: 0 accepted, 1 invalid operation, 2 payload too large, 3 denied, 4 capacity. */
bool ghostos_embedded_limits(size_t max_source_bytes, uint64_t max_operations, size_t max_call_levels,
    size_t max_expression_depth, size_t max_variables, size_t max_functions, size_t max_modules,
    size_t max_string_bytes, size_t max_array_items, size_t max_map_items, size_t max_requests,
    size_t max_request_bytes);
int ghostos_embedded_capability(const uint8_t *name, size_t length, uint64_t operations);
bool ghostos_embedded_operation_mask(uint8_t operation, uint64_t *mask);
bool ghostos_embedded_allows(uint64_t operations, uint8_t operation);
int ghostos_embedded_names(const uint8_t *const *names, const size_t *lengths, size_t count, size_t capacity);
bool ghostos_embedded_source(size_t length, size_t max_source_bytes);
int ghostos_embedded_request(bool operation_ok, uint8_t operation, bool allowed, size_t payload_length,
    size_t max_payload, size_t requests, size_t max_requests, uint32_t *sequence);
#endif
