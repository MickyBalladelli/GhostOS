#ifndef GHOSTOS_VM_VIRTIO_H
#define GHOSTOS_VM_VIRTIO_H

#include "ghostos/vm_virtio_queue.h"

#define GHOSTOS_VM_VIRTIO_BLOCK 0u
#define GHOSTOS_VM_VIRTIO_CONSOLE 1u
#define GHOSTOS_VM_VIRTIO_RNG 2u
#define GHOSTOS_VM_VIRTIO_MAX_DMA_BYTES (16u * 1024u * 1024u)

typedef struct {
    uint32_t guest_features, pfn;
    uint16_t avail_last, used_idx, queue_sel;
    uint8_t status, interrupt_status;
    bool pending;
} ghostos_vm_virtio_transport;

typedef struct ghostos_vm_virtio_console ghostos_vm_virtio_console;
typedef struct {
    ghostos_vm_virtio_read_fn read_memory;
    ghostos_vm_virtio_write_fn write_memory;
    bool (*read_sector)(void *context, uint64_t sector, uint8_t *bytes);
    bool (*write_sector)(void *context, uint64_t sector, const uint8_t *bytes);
    bool (*flush_disk)(void *context);
    bool (*entropy)(void *context, uint8_t *bytes, size_t length);
    void (*console)(void *context, const uint8_t *bytes, size_t length);
    void (*interrupt)(void *context);
    void *context;
} ghostos_vm_virtio_io;

void ghostos_vm_virtio_transport_init(ghostos_vm_virtio_transport *transport);
bool ghostos_vm_virtio_read_common(ghostos_vm_virtio_transport *transport,
    uint16_t offset, uint32_t features, uint64_t *value);
bool ghostos_vm_virtio_write_common(ghostos_vm_virtio_transport *transport,
    uint16_t offset, uint32_t value);
bool ghostos_vm_virtio_take_pending(ghostos_vm_virtio_transport *transport);
/* Device port results: 0 success, 1 unsupported size, 2 invalid address.
 * Common-register reads intentionally retain legacy size-independent access. */
uint8_t ghostos_vm_virtio_device_read(ghostos_vm_virtio_transport *transport,
    uint8_t kind, uint16_t port, uint8_t size, uint64_t sectors, uint64_t *value);
uint8_t ghostos_vm_virtio_device_write(ghostos_vm_virtio_transport *transport,
    uint16_t port, uint64_t value, uint8_t size);

ghostos_vm_virtio_console *ghostos_vm_virtio_console_new(void);
void ghostos_vm_virtio_console_free(ghostos_vm_virtio_console *console);
void ghostos_vm_virtio_console_reset(ghostos_vm_virtio_console *console);
const uint8_t *ghostos_vm_virtio_console_output(const ghostos_vm_virtio_console *console,
    size_t *length);
void ghostos_vm_virtio_console_clear_output(ghostos_vm_virtio_console *console);
bool ghostos_vm_virtio_console_append(ghostos_vm_virtio_console *console,
    const uint8_t *bytes, size_t length);
void ghostos_vm_virtio_fill_random(uint64_t *fallback_state, uint8_t *bytes,
    size_t length, const ghostos_vm_virtio_io *io);
/* Poll returns false only on host allocation failure. Guest memory/disk
 * errors produce the existing device completion statuses. Callbacks run
 * synchronously, are not retained, and must not reenter the device. */
bool ghostos_vm_virtio_block_poll(ghostos_vm_virtio_transport *transport,
    const ghostos_vm_virtio_io *io);
bool ghostos_vm_virtio_console_poll(ghostos_vm_virtio_transport *transport,
    ghostos_vm_virtio_console *console, const ghostos_vm_virtio_io *io);
bool ghostos_vm_virtio_rng_poll(ghostos_vm_virtio_transport *transport,
    uint64_t *fallback_state, const ghostos_vm_virtio_io *io);

#endif
