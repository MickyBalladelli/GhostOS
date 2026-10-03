#include "ghostos/enclave.h"
enum { GHOSTOS_ENCLAVE_PAGE = 4096 };
int ghostos_enclave_binding(const uint8_t measurement[32], size_t range_count, size_t capacity,
    const uint64_t *starts, const uint64_t *lengths) {
    size_t i;
    bool zero = true;
    for (i = 0; i < 32; ++i) if (measurement[i]) zero = false;
    if (zero || !range_count || range_count > capacity || !capacity) return 1;
    for (i = 0; i < range_count; ++i) {
        if (starts[i] % GHOSTOS_ENCLAVE_PAGE || lengths[i] % GHOSTOS_ENCLAVE_PAGE || !lengths[i] ||
            lengths[i] > UINT64_MAX - starts[i]) return 1;
    }
    return 0;
}
int ghostos_enclave_register(const uint32_t *nodes, const bool *occupied, size_t count, uint32_t node, size_t *index) {
    size_t free_slot = count;
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!occupied[i]) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (nodes[i] == node) return 2;
    }
    if (free_slot == count) return 3;
    *index = free_slot;
    return 0;
}
bool ghostos_enclave_protects(const uint64_t *starts, const uint64_t *lengths, size_t count,
    uint64_t start, uint64_t length) {
    uint64_t end;
    size_t i;
    if (length > UINT64_MAX - start) return false;
    end = start + length;
    for (i = 0; i < count; ++i) {
        if (starts[i] <= start && starts[i] + lengths[i] >= end) return true;
    }
    return false;
}
uint8_t ghostos_enclave_class(uint8_t platform) {
    return platform == 2 ? 1 : 0;
}
