#ifndef GHOSTOS_VM_VIRTIO_NET_H
#define GHOSTOS_VM_VIRTIO_NET_H

#include "ghostos/vm_virtio_queue.h"

typedef struct ghostos_vm_virtio_net ghostos_vm_virtio_net;
typedef struct {
    ghostos_vm_virtio_read_fn read_memory;
    ghostos_vm_virtio_write_fn write_memory;
    /* Receive: 1 packet, 0 empty, -(NetError + 1) failure. The packet view
     * remains live until the next receive call. Transmit: -1 success, or
     * NetError (0..5) failure. Missing backend uses BackendUnavailable (5). */
    int32_t (*receive)(void *context, const uint8_t **packet, size_t *length);
    int32_t (*transmit)(void *context, const uint8_t *packet, size_t length);
    void (*interrupt)(void *context);
    void *context;
} ghostos_vm_virtio_net_io;

ghostos_vm_virtio_net *ghostos_vm_virtio_net_new(const uint8_t mac[6]);
void ghostos_vm_virtio_net_free(ghostos_vm_virtio_net *net);
void ghostos_vm_virtio_net_reset(ghostos_vm_virtio_net *net);
void ghostos_vm_virtio_net_clear_rx(ghostos_vm_virtio_net *net);
bool ghostos_vm_virtio_net_pending(const ghostos_vm_virtio_net *net);
void ghostos_vm_virtio_net_notify(ghostos_vm_virtio_net *net);
int32_t ghostos_vm_virtio_net_take_error(ghostos_vm_virtio_net *net);
uint64_t ghostos_vm_virtio_net_read(ghostos_vm_virtio_net *net, uint16_t port, bool carrier);
void ghostos_vm_virtio_net_write(ghostos_vm_virtio_net *net, uint16_t port, uint32_t value);
/* False means host allocation failed. Synchronous callbacks are never
 * retained and must not reenter this controller. This preserves the VM's
 * existing direct-index descriptor traversal and shared queue PFN behavior. */
bool ghostos_vm_virtio_net_poll(ghostos_vm_virtio_net *net, const ghostos_vm_virtio_net_io *io);

#endif
