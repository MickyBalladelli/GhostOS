#ifndef GHOSTOS_ENCLAVE_H
#define GHOSTOS_ENCLAVE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Binding and register: 0 accepted, 1 invalid, 2 duplicate, 3 capacity.
 * Platform class: CPU=0, GPU=1. */
int ghostos_enclave_binding(const uint8_t measurement[32], size_t range_count, size_t capacity,
    const uint64_t *starts, const uint64_t *lengths);
int ghostos_enclave_register(const uint32_t *nodes, const bool *occupied, size_t count, uint32_t node, size_t *index);
bool ghostos_enclave_protects(const uint64_t *starts, const uint64_t *lengths, size_t count,
    uint64_t start, uint64_t length);
uint8_t ghostos_enclave_class(uint8_t platform);
#endif
