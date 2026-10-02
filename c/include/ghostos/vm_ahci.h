#ifndef GHOSTOS_VM_AHCI_H
#define GHOSTOS_VM_AHCI_H

#include "ghostos/vm_storage_io.h"

typedef struct ghostos_vm_ahci ghostos_vm_ahci;
typedef struct { uint64_t address; size_t length; } ghostos_vm_ahci_prd;
enum { GHOSTOS_VM_AHCI_OK = 0, GHOSTOS_VM_AHCI_SIZE = 1,
    GHOSTOS_VM_AHCI_DMA = 2, GHOSTOS_VM_AHCI_DISK = 3,
    GHOSTOS_VM_AHCI_LEGACY_BOUNDS = 4, GHOSTOS_VM_AHCI_LEGACY_OVERFLOW = 5 };

ghostos_vm_ahci *ghostos_vm_ahci_new(bool checked_arithmetic);
void ghostos_vm_ahci_free(ghostos_vm_ahci *ahci);
void ghostos_vm_ahci_reset(ghostos_vm_ahci *ahci);
bool ghostos_vm_ahci_pending(const ghostos_vm_ahci *ahci);
uint32_t ghostos_vm_ahci_read(const ghostos_vm_ahci *ahci, uint64_t address, uint8_t size, uint64_t *value);
uint32_t ghostos_vm_ahci_write(ghostos_vm_ahci *ahci, uint64_t address, uint8_t size, uint32_t value);
/* Poll reports only host legacy panic conditions. Guest DMA/disk failures
 * update TFD/SERR and completion interrupts inside the controller. */
uint32_t ghostos_vm_ahci_poll(ghostos_vm_ahci *ahci, const ghostos_vm_storage_io *io);
void ghostos_vm_ahci_identify(uint64_t sectors, uint8_t output[512]);
/* Same sector-chunk rules as the existing controller. A PRD crossing a
 * sector boundary returns LEGACY_BOUNDS instead of accessing outside it. */
uint32_t ghostos_vm_ahci_transfer(ghostos_vm_ahci *ahci, const ghostos_vm_storage_io *io, uint64_t lba,
    size_t sectors, const ghostos_vm_ahci_prd *prds, size_t prd_count, bool to_disk);

#endif
