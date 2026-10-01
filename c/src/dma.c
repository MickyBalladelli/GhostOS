#include "ghostos/dma.h"

static bool range_valid(uint64_t offset, uint64_t length) {
    return offset % GHOSTOS_DMA_PAGE_SIZE == 0 && length != 0 &&
        length % GHOSTOS_DMA_PAGE_SIZE == 0 && UINT64_MAX - offset >= length;
}

static bool align_iova(uint64_t value, uint64_t *out) {
    if (UINT64_MAX - value < GHOSTOS_DMA_PAGE_SIZE - 1) return false;
    *out = (value + GHOSTOS_DMA_PAGE_SIZE - 1) & ~(GHOSTOS_DMA_PAGE_SIZE - 1);
    return true;
}

static uint16_t required_rights(uint8_t permissions) {
    uint16_t rights = 0;
    if (permissions & GHOSTOS_DMA_DEVICE_READ) rights |= GHOSTOS_RIGHT_DMA_READ;
    if (permissions & GHOSTOS_DMA_DEVICE_WRITE) rights |= GHOSTOS_RIGHT_DMA_WRITE;
    return rights;
}

void ghostos_dma_init(ghostos_dma_manager *manager, size_t capacity) {
    if (capacity > GHOSTOS_MAX_DMA_MAPPINGS) capacity = GHOSTOS_MAX_DMA_MAPPINGS;
    manager->next_id = 1;
    manager->next_iova = GHOSTOS_DMA_PAGE_SIZE;
    manager->capacity = capacity;
    for (size_t i = 0; i < GHOSTOS_MAX_DMA_MAPPINGS; ++i)
        manager->mappings[i] = (ghostos_dma_record){0};
}

ghostos_dma_error ghostos_dma_map(ghostos_dma_manager *manager,
    const ghostos_capability_space *capabilities, uint32_t caller,
    ghostos_capability_handle device_authority, ghostos_capability_handle buffer_authority,
    uint32_t device, uint64_t offset, uint64_t length, uint8_t permissions,
    ghostos_iommu iommu, ghostos_dma_mapping *out) {
    if (!range_valid(offset, length)) return GHOSTOS_DMA_INVALID_RANGE;
    if ((permissions & ~(GHOSTOS_DMA_DEVICE_READ | GHOSTOS_DMA_DEVICE_WRITE)) != 0)
        return GHOSTOS_DMA_INVALID_PERMISSIONS;
    uint16_t required = required_rights(permissions);
    if (!required) return GHOSTOS_DMA_INVALID_PERMISSIONS;
    ghostos_capability_object device_object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_DMA_DEVICE, 0, 0, device, 0);
    if (ghostos_capability_authorize(capabilities, caller, device_authority,
            device_object, required) != GHOSTOS_CAP_OK) return GHOSTOS_DMA_INVALID_CAPABILITY;

    ghostos_capability_info buffer;
    if (ghostos_capability_inspect(capabilities, caller, buffer_authority, &buffer) != GHOSTOS_CAP_OK)
        return GHOSTOS_DMA_INVALID_CAPABILITY;
    if ((buffer.rights & (GHOSTOS_RIGHT_MAP | required)) != (GHOSTOS_RIGHT_MAP | required))
        return GHOSTOS_DMA_ACCESS_DENIED;
    if (!buffer.has_backing) return GHOSTOS_DMA_INVALID_CAPABILITY;
    ghostos_capability_range backing = buffer.backing;
    if (backing.start % GHOSTOS_DMA_PAGE_SIZE != 0) return GHOSTOS_DMA_INVALID_CAPABILITY;
    if (UINT64_MAX - backing.start < offset) return GHOSTOS_DMA_INVALID_RANGE;
    ghostos_capability_range physical;
    if (!ghostos_capability_range_new(backing.start + offset, length, &physical))
        return GHOSTOS_DMA_INVALID_RANGE;
    if (!ghostos_capability_range_contains(backing, physical)) return GHOSTOS_DMA_ACCESS_DENIED;

    size_t slot = manager->capacity;
    for (size_t i = 0; i < manager->capacity; ++i)
        if (!manager->mappings[i].occupied) { slot = i; break; }
    if (slot == manager->capacity) return GHOSTOS_DMA_CAPACITY;
    uint64_t iova, next;
    if (!align_iova(manager->next_iova, &iova) || UINT64_MAX - iova < length)
        return GHOSTOS_DMA_IOVA_EXHAUSTED;
    next = iova + length;
    if (!iommu.map || !iommu.map(iommu.context, device, iova, physical, permissions))
        return GHOSTOS_DMA_IOMMU_REJECTED;

    ghostos_dma_mapping mapping = {manager->next_id, device, iova, physical, permissions};
    manager->next_id++;
    if (manager->next_id == 0) manager->next_id = 1;
    manager->next_iova = next;
    manager->mappings[slot] = (ghostos_dma_record){mapping, caller, device_authority, buffer_authority, true};
    if (out) *out = mapping;
    return GHOSTOS_DMA_OK;
}

ghostos_dma_error ghostos_dma_unmap(ghostos_dma_manager *manager,
    const ghostos_capability_space *capabilities, uint32_t caller,
    ghostos_capability_handle device_authority, ghostos_capability_handle buffer_authority,
    uint64_t id, ghostos_iommu iommu, ghostos_dma_mapping *out) {
    size_t slot = manager->capacity;
    for (size_t i = 0; i < manager->capacity; ++i)
        if (manager->mappings[i].occupied && manager->mappings[i].mapping.id == id) { slot = i; break; }
    if (slot == manager->capacity) return GHOSTOS_DMA_MAPPING_NOT_FOUND;
    ghostos_dma_record record = manager->mappings[slot];
    if (record.owner != caller || record.device_authority != device_authority ||
        record.buffer_authority != buffer_authority) return GHOSTOS_DMA_ACCESS_DENIED;
    uint16_t required = required_rights(record.mapping.permissions);
    ghostos_capability_object device_object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_DMA_DEVICE, 0, 0, record.mapping.device, 0);
    if (!required || ghostos_capability_authorize(capabilities, caller, device_authority,
            device_object, required) != GHOSTOS_CAP_OK) return GHOSTOS_DMA_INVALID_CAPABILITY;
    ghostos_capability_info buffer;
    if (ghostos_capability_inspect(capabilities, caller, buffer_authority, &buffer) != GHOSTOS_CAP_OK)
        return GHOSTOS_DMA_INVALID_CAPABILITY;
    if (!iommu.unmap || !iommu.unmap(iommu.context, record.mapping.device,
            record.mapping.iova, record.mapping.physical.length)) return GHOSTOS_DMA_IOMMU_REJECTED;
    manager->mappings[slot].occupied = false;
    if (out) *out = record.mapping;
    return GHOSTOS_DMA_OK;
}

bool ghostos_dma_mapping_get(const ghostos_dma_manager *manager, uint64_t id, ghostos_dma_mapping *out) {
    for (size_t i = 0; i < manager->capacity; ++i) {
        if (manager->mappings[i].occupied && manager->mappings[i].mapping.id == id) {
            if (out) *out = manager->mappings[i].mapping;
            return true;
        }
    }
    return false;
}
