#include "ghostos/embedded_script.h"
bool ghostos_embedded_limits(size_t max_source_bytes, uint64_t max_operations, size_t max_call_levels,
    size_t max_expression_depth, size_t max_variables, size_t max_functions, size_t max_modules,
    size_t max_string_bytes, size_t max_array_items, size_t max_map_items, size_t max_requests,
    size_t max_request_bytes) {
    return max_source_bytes && max_operations && max_call_levels && max_expression_depth &&
        max_variables && max_functions && max_modules && max_string_bytes && max_array_items &&
        max_map_items && max_requests && max_request_bytes;
}
int ghostos_embedded_capability(const uint8_t *name, size_t length, uint64_t operations) {
    size_t i;
    if (!length || length > 64 || !operations) return 1;
    for (i = 0; i < length; ++i) if (!name[i]) return 1;
    return 0;
}
bool ghostos_embedded_operation_mask(uint8_t operation, uint64_t *mask) {
    if (operation >= 64) return false;
    *mask = UINT64_C(1) << operation;
    return true;
}
bool ghostos_embedded_allows(uint64_t operations, uint8_t operation) {
    return operation < 64 && (operations & (UINT64_C(1) << operation)) != 0;
}
static bool same_name(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
int ghostos_embedded_names(const uint8_t *const *names, const size_t *lengths, size_t count, size_t capacity) {
    size_t i, j;
    if (count > capacity) return 1;
    for (i = 0; i < count; ++i) {
        for (j = 0; j < i; ++j) if (same_name(names[j], lengths[j], names[i], lengths[i])) return 2;
    }
    return 0;
}
bool ghostos_embedded_source(size_t length, size_t max_source_bytes) {
    return length <= max_source_bytes;
}
int ghostos_embedded_request(bool operation_ok, uint8_t operation, bool allowed, size_t payload_length,
    size_t max_payload, size_t requests, size_t max_requests, uint32_t *sequence) {
    if (!operation_ok || operation >= 64) return 1;
    if (payload_length > max_payload) return 2;
    if (!allowed) return 3;
    if (requests >= max_requests || requests >= UINT32_MAX) return 4;
    *sequence = (uint32_t)(requests + 1);
    return 0;
}
