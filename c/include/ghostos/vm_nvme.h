#ifndef GHOSTOS_VM_NVME_H
#define GHOSTOS_VM_NVME_H

#include "ghostos/vm_storage_io.h"

typedef struct ghostos_vm_nvme ghostos_vm_nvme;
enum {
    GHOSTOS_VM_NVME_OK = 0,
    GHOSTOS_VM_NVME_SIZE = 1,
    GHOSTOS_VM_NVME_ALLOCATION = 2,
    GHOSTOS_VM_NVME_LEGACY_OVERFLOW = 3
};

ghostos_vm_nvme *ghostos_vm_nvme_new(bool checked_arithmetic);
void ghostos_vm_nvme_free(ghostos_vm_nvme *nvme);
void ghostos_vm_nvme_reset(ghostos_vm_nvme *nvme);
bool ghostos_vm_nvme_pending(const ghostos_vm_nvme *nvme);
uint32_t ghostos_vm_nvme_read(const ghostos_vm_nvme *nvme, uint64_t address, uint8_t size, uint64_t *value);
uint32_t ghostos_vm_nvme_write(ghostos_vm_nvme *nvme, uint64_t address, uint8_t size, uint64_t value);
uint32_t ghostos_vm_nvme_poll(ghostos_vm_nvme *nvme, const ghostos_vm_storage_io *io);
/* API result is separate from the guest completion status. Preserve the
 * existing command layout, including PRP1 at byte 8 and SQ CQID at byte 48. */
uint32_t ghostos_vm_nvme_admin(ghostos_vm_nvme *nvme, const ghostos_vm_storage_io *io, const uint8_t command[64], uint32_t *status);
uint32_t ghostos_vm_nvme_command(ghostos_vm_nvme *nvme, const ghostos_vm_storage_io *io, const uint8_t command[64], uint32_t *status);

#endif
