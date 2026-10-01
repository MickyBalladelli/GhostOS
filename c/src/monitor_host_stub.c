#include "ghostos/dlm.h"

bool ghostos_dlm_kernel_lock_summary(size_t index, uint64_t *resource,
    uint32_t *owner_node, uint32_t *address_space, uint32_t *mode, bool *granted,
    uint64_t *requested_at_us, uint64_t *granted_at_us) {
    (void)index;
    (void)resource;
    (void)owner_node;
    (void)address_space;
    (void)mode;
    (void)granted;
    (void)requested_at_us;
    (void)granted_at_us;
    return false;
}
