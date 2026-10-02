#include "ghostos/vm_virtio_queue.h"

uint64_t ghostos_vm_virtio_desc_base(uint32_t pfn) {
    return (uint64_t)pfn << 12;
}

uint64_t ghostos_vm_virtio_avail_base(uint32_t pfn) {
    return ghostos_vm_virtio_desc_base(pfn) +
        GHOSTOS_VM_VIRTIO_QUEUE_SIZE * GHOSTOS_VM_VIRTIO_DESC_SIZE;
}

uint64_t ghostos_vm_virtio_used_base(uint32_t pfn) {
    uint64_t avail_end = ghostos_vm_virtio_avail_base(pfn) + 4u +
        GHOSTOS_VM_VIRTIO_QUEUE_SIZE * 2u;
    return (avail_end + 3u) & ~UINT64_C(3);
}

static bool read_u16(ghostos_vm_virtio_read_fn read, void *context,
    uint64_t address, uint16_t *value) {
    uint8_t bytes[2];
    if (!read || !read(context, address, bytes, sizeof(bytes))) return false;
    *value = (uint16_t)(bytes[0] | ((uint16_t)bytes[1] << 8));
    return true;
}

bool ghostos_vm_virtio_next_available(uint32_t pfn, uint16_t *avail_last,
    ghostos_vm_virtio_read_fn read, void *context, uint16_t *head) {
    if (pfn == 0 || !avail_last || !read || !head) return false;
    uint16_t avail_index;
    if (!read_u16(read, context, ghostos_vm_virtio_avail_base(pfn) + 2u,
            &avail_index) || *avail_last == avail_index) return false;
    if ((uint16_t)(avail_index - *avail_last) > GHOSTOS_VM_VIRTIO_QUEUE_SIZE) {
        *avail_last = avail_index;
        return false;
    }
    uint64_t slot = *avail_last & (GHOSTOS_VM_VIRTIO_QUEUE_SIZE - 1u);
    if (!read_u16(read, context,
            ghostos_vm_virtio_avail_base(pfn) + 4u + slot * 2u, head)) return false;
    *avail_last = (uint16_t)(*avail_last + 1u);
    return true;
}

bool ghostos_vm_virtio_descriptor_chain(uint32_t pfn, uint16_t head,
    ghostos_vm_virtio_read_fn read, void *context,
    ghostos_vm_virtio_descriptor output[GHOSTOS_VM_VIRTIO_MAX_CHAIN],
    size_t *count) {
    if (head >= GHOSTOS_VM_VIRTIO_QUEUE_SIZE || !read || !output || !count)
        return false;
    *count = 0;
    uint16_t index = head;
    for (size_t i = 0; i < GHOSTOS_VM_VIRTIO_MAX_CHAIN; ++i) {
        uint64_t address = ghostos_vm_virtio_desc_base(pfn) +
            (uint64_t)index * GHOSTOS_VM_VIRTIO_DESC_SIZE;
        uint8_t bytes[GHOSTOS_VM_VIRTIO_DESC_SIZE];
        if (!read(context, address, bytes, sizeof(bytes))) return false;
        ghostos_vm_virtio_descriptor descriptor = {
            (uint64_t)bytes[0] | ((uint64_t)bytes[1] << 8) |
                ((uint64_t)bytes[2] << 16) | ((uint64_t)bytes[3] << 24) |
                ((uint64_t)bytes[4] << 32) | ((uint64_t)bytes[5] << 40) |
                ((uint64_t)bytes[6] << 48) | ((uint64_t)bytes[7] << 56),
            (uint32_t)bytes[8] | ((uint32_t)bytes[9] << 8) |
                ((uint32_t)bytes[10] << 16) | ((uint32_t)bytes[11] << 24),
            (uint16_t)(bytes[12] | ((uint16_t)bytes[13] << 8)),
            (uint16_t)(bytes[14] | ((uint16_t)bytes[15] << 8))
        };
        if ((descriptor.flags & ~(GHOSTOS_VM_VIRTIO_DESC_NEXT |
                GHOSTOS_VM_VIRTIO_DESC_WRITE | GHOSTOS_VM_VIRTIO_DESC_INDIRECT)) != 0 ||
            (descriptor.flags & GHOSTOS_VM_VIRTIO_DESC_INDIRECT) != 0) return false;
        output[i] = descriptor;
        *count = i + 1;
        if ((descriptor.flags & GHOSTOS_VM_VIRTIO_DESC_NEXT) == 0) return true;
        index = descriptor.next;
        if (index >= GHOSTOS_VM_VIRTIO_QUEUE_SIZE) return false;
    }
    return false;
}

bool ghostos_vm_virtio_complete(uint32_t pfn, uint16_t *used_idx,
    uint16_t head, uint32_t length, ghostos_vm_virtio_write_fn write,
    void *context) {
    if (!used_idx || !write) return false;
    uint64_t slot = *used_idx & (GHOSTOS_VM_VIRTIO_QUEUE_SIZE - 1u);
    uint64_t entry = ghostos_vm_virtio_used_base(pfn) + 4u + slot * 8u;
    uint8_t bytes[8] = {
        (uint8_t)head, (uint8_t)(head >> 8), 0, 0,
        (uint8_t)length, (uint8_t)(length >> 8),
        (uint8_t)(length >> 16), (uint8_t)(length >> 24)
    };
    if (!write(context, entry, bytes, sizeof(bytes))) return false;
    *used_idx = (uint16_t)(*used_idx + 1u);
    uint8_t index_bytes[2] = {(uint8_t)*used_idx, (uint8_t)(*used_idx >> 8)};
    return write(context, ghostos_vm_virtio_used_base(pfn) + 2u,
        index_bytes, sizeof(index_bytes));
}
