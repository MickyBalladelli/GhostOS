#ifndef GHOSTOS_VM_E1000_H
#define GHOSTOS_VM_E1000_H
#include "ghostos/vm_virtio_net.h"

#define GHOSTOS_VM_E1000_MMIO_SIZE UINT64_C(0x20000)
typedef struct ghostos_vm_e1000 ghostos_vm_e1000;
/* Both emulated NICs use the same memory/backend/IRQ callback contract. */
typedef ghostos_vm_virtio_net_io ghostos_vm_e1000_io;
enum ghostos_vm_e1000_result {
    GHOSTOS_VM_E1000_OK, GHOSTOS_VM_E1000_SIZE, GHOSTOS_VM_E1000_LEGACY_OVERFLOW
};
ghostos_vm_e1000 *ghostos_vm_e1000_new(const uint8_t[6]);
void ghostos_vm_e1000_free(ghostos_vm_e1000 *);
void ghostos_vm_e1000_reset(ghostos_vm_e1000 *);
void ghostos_vm_e1000_clear_rx(ghostos_vm_e1000 *);
int32_t ghostos_vm_e1000_take_error(ghostos_vm_e1000 *);
uint32_t ghostos_vm_e1000_read(const ghostos_vm_e1000 *, uint64_t, uint8_t, bool, uint64_t *);
uint32_t ghostos_vm_e1000_write(ghostos_vm_e1000 *, uint64_t, uint8_t, uint32_t,
    const ghostos_vm_e1000_io *);
void ghostos_vm_e1000_signal(ghostos_vm_e1000 *, uint32_t, const ghostos_vm_e1000_io *);
/* Keep existing ring indexing, ignored status-write failures, interrupt masks,
 * and debug-build overflow behavior. Callbacks are synchronous, not retained. */
uint32_t ghostos_vm_e1000_poll(ghostos_vm_e1000 *, const ghostos_vm_e1000_io *, bool);
#endif
