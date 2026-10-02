#ifndef GHOSTOS_VM_VIRTIO_QUEUE_H
#define GHOSTOS_VM_VIRTIO_QUEUE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_VIRTIO_QUEUE_SIZE 128u
#define GHOSTOS_VM_VIRTIO_DESC_SIZE 16u
#define GHOSTOS_VM_VIRTIO_MAX_CHAIN 64u
#define GHOSTOS_VM_VIRTIO_DESC_NEXT 1u
#define GHOSTOS_VM_VIRTIO_DESC_WRITE 2u
#define GHOSTOS_VM_VIRTIO_DESC_INDIRECT 4u

typedef struct {
    uint64_t addr;
    uint32_t len;
    uint16_t flags;
    uint16_t next;
} ghostos_vm_virtio_descriptor;

typedef bool (*ghostos_vm_virtio_read_fn)(void *context, uint64_t address,
    uint8_t *output, size_t length);
typedef bool (*ghostos_vm_virtio_write_fn)(void *context, uint64_t address,
    const uint8_t *input, size_t length);

uint64_t ghostos_vm_virtio_desc_base(uint32_t pfn);
uint64_t ghostos_vm_virtio_avail_base(uint32_t pfn);
uint64_t ghostos_vm_virtio_used_base(uint32_t pfn);
bool ghostos_vm_virtio_next_available(uint32_t pfn, uint16_t *avail_last,
    ghostos_vm_virtio_read_fn read, void *context, uint16_t *head);
bool ghostos_vm_virtio_descriptor_chain(uint32_t pfn, uint16_t head,
    ghostos_vm_virtio_read_fn read, void *context,
    ghostos_vm_virtio_descriptor output[GHOSTOS_VM_VIRTIO_MAX_CHAIN],
    size_t *count);
bool ghostos_vm_virtio_complete(uint32_t pfn, uint16_t *used_idx,
    uint16_t head, uint32_t length, ghostos_vm_virtio_write_fn write,
    void *context);

#endif
