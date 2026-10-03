#include "ghostos/dma.h"

_Static_assert(sizeof(ghostos_dma_mapping) == 48, "DMA mapping ABI");
_Static_assert(offsetof(ghostos_dma_mapping, physical) == 24, "DMA physical ABI");
_Static_assert(sizeof(ghostos_dma_record) == 80, "DMA record ABI");
_Static_assert(offsetof(ghostos_dma_record, occupied) == 72, "DMA occupancy ABI");
_Static_assert(sizeof(ghostos_dma_plan) == 96, "DMA plan ABI");
_Static_assert(sizeof(ghostos_dma_state) == 16, "DMA state ABI");

static uint16_t permission_rights(uint8_t permissions) {
    uint16_t rights = 0;
    if (permissions & GHOSTOS_DMA_DEVICE_READ) rights |= GHOSTOS_RIGHT_DMA_READ;
    if (permissions & GHOSTOS_DMA_DEVICE_WRITE) rights |= GHOSTOS_RIGHT_DMA_WRITE;
    return rights;
}

ghostos_dma_error ghostos_dma_request(uint64_t offset, uint64_t length,
    uint8_t permissions, uint16_t *rights) {
    if (offset % GHOSTOS_DMA_PAGE_SIZE || !length || length % GHOSTOS_DMA_PAGE_SIZE ||
        UINT64_MAX - offset < length) return GHOSTOS_DMA_INVALID_RANGE;
    *rights = permission_rights(permissions);
    return *rights ? GHOSTOS_DMA_OK : GHOSTOS_DMA_INVALID_PERMISSIONS;
}

ghostos_dma_error ghostos_dma_buffer(uint16_t rights, uint16_t required,
    bool has_backing, ghostos_capability_range backing, uint64_t offset,
    uint64_t length, ghostos_capability_range *physical) {
    uint16_t needed = GHOSTOS_RIGHT_MAP | required;
    if ((rights & needed) != needed) return GHOSTOS_DMA_ACCESS_DENIED;
    if (!has_backing || backing.start % GHOSTOS_DMA_PAGE_SIZE)
        return GHOSTOS_DMA_INVALID_CAPABILITY;
    if (UINT64_MAX - backing.start < offset) return GHOSTOS_DMA_INVALID_RANGE;
    uint64_t start = backing.start + offset;
    /* Rust maps a failed PhysicalRange constructor to AccessDenied here. */
    if (!length || UINT64_MAX - start < length) return GHOSTOS_DMA_ACCESS_DENIED;
    uint64_t backing_end = UINT64_MAX - backing.start < backing.length ?
        UINT64_MAX : backing.start + backing.length;
    if (start < backing.start || start + length > backing_end) return GHOSTOS_DMA_ACCESS_DENIED;
    *physical = (ghostos_capability_range){start, length};
    return GHOSTOS_DMA_OK;
}

ghostos_dma_error ghostos_dma_prepare_map(const ghostos_dma_state *state,
    const ghostos_dma_record *records, size_t capacity, uint32_t caller,
    uint64_t device_authority, uint64_t buffer_authority, uint32_t device,
    ghostos_capability_range physical, uint8_t permissions, ghostos_dma_plan *plan) {
    size_t slot = 0;
    while (slot < capacity && records[slot].occupied) ++slot;
    if (slot == capacity) return GHOSTOS_DMA_CAPACITY;
    if (UINT64_MAX - state->next_iova < GHOSTOS_DMA_PAGE_SIZE - 1)
        return GHOSTOS_DMA_IOVA_EXHAUSTED;
    uint64_t iova = (state->next_iova + GHOSTOS_DMA_PAGE_SIZE - 1) & ~(GHOSTOS_DMA_PAGE_SIZE - 1);
    if (UINT64_MAX - iova < physical.length) return GHOSTOS_DMA_IOVA_EXHAUSTED;
    *plan = (ghostos_dma_plan){
        {{state->next_id, device, iova, physical, permissions}, caller,
            device_authority, buffer_authority, true}, slot, iova + physical.length};
    return GHOSTOS_DMA_OK;
}

void ghostos_dma_commit_map(ghostos_dma_state *state, ghostos_dma_record *records,
    const ghostos_dma_plan *plan) {
    ++state->next_id;
    if (!state->next_id) state->next_id = 1;
    state->next_iova = plan->next_iova;
    records[plan->slot] = plan->record;
}

ghostos_dma_error ghostos_dma_prepare_unmap(const ghostos_dma_record *records,
    size_t capacity, uint32_t caller, uint64_t device_authority,
    uint64_t buffer_authority, uint64_t id, ghostos_dma_plan *plan, uint16_t *rights) {
    size_t slot = 0;
    while (slot < capacity && (!records[slot].occupied || records[slot].mapping.id != id)) ++slot;
    if (slot == capacity) return GHOSTOS_DMA_MAPPING_NOT_FOUND;
    ghostos_dma_record record = records[slot];
    if (record.owner != caller || record.device_authority != device_authority ||
        record.buffer_authority != buffer_authority) return GHOSTOS_DMA_ACCESS_DENIED;
    *rights = permission_rights(record.mapping.permissions);
    if (!*rights) return GHOSTOS_DMA_INVALID_PERMISSIONS;
    *plan = (ghostos_dma_plan){record, slot, 0};
    return GHOSTOS_DMA_OK;
}

void ghostos_dma_commit_unmap(ghostos_dma_record *records, const ghostos_dma_plan *plan) {
    records[plan->slot] = (ghostos_dma_record){0};
}

bool ghostos_dma_records_get(const ghostos_dma_record *records, size_t capacity,
    uint64_t id, ghostos_dma_mapping *mapping) {
    for (size_t i = 0; i < capacity; ++i) {
        if (records[i].occupied && records[i].mapping.id == id) {
            *mapping = records[i].mapping;
            return true;
        }
    }
    return false;
}
