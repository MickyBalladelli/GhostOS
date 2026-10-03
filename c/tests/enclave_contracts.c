#include "ghostos/enclave.h"
#include <assert.h>
static void admission_protects_only_registered_pages(void) {
    uint8_t measurement[32];
    uint32_t nodes[2] = {0, 0};
    bool occupied[2] = {false, false};
    uint64_t start = 4096u * 4u;
    uint64_t length = 4096;
    size_t index = 9;
    size_t i;
    for (i = 0; i < 32; ++i) measurement[i] = 7;
    assert(ghostos_enclave_binding(measurement, 1, 2, &start, &length) == 0);
    measurement[0] = 0;
    for (i = 1; i < 32; ++i) measurement[i] = 0;
    assert(ghostos_enclave_binding(measurement, 1, 2, &start, &length) == 1);
    for (i = 0; i < 32; ++i) measurement[i] = 7;
    start = 100;
    assert(ghostos_enclave_binding(measurement, 1, 2, &start, &length) == 1);
    start = 4096u * 4u;
    assert(!ghostos_enclave_register(nodes, occupied, 2, 2, &index) && index == 0);
    nodes[0] = 2;
    occupied[0] = true;
    assert(ghostos_enclave_register(nodes, occupied, 2, 2, &index) == 2);
    assert(ghostos_enclave_protects(&start, &length, 1, start, length));
    assert(!ghostos_enclave_protects(&start, &length, 1, 4096u * 8u, length));
    assert(!ghostos_enclave_protects(&start, &length, 1, UINT64_MAX, 2));
    assert(ghostos_enclave_class(0) == 0 && ghostos_enclave_class(2) == 1);
}
int main(void) {
    admission_protects_only_registered_pages();
    return 0;
}
