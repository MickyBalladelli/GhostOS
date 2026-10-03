#include "ghostos/driver_capabilities.h"

void ghostos_driver_grant_empty(ghostos_driver_grant *grant) {
    *grant = (ghostos_driver_grant){0};
    grant->manifest.magic = GHOSTOS_SERVICE_RESOURCE_MAGIC;
    grant->manifest.version = GHOSTOS_SERVICE_RESOURCE_VERSION;
}

ghostos_grant_error ghostos_driver_grant_resources(ghostos_capability_space *capabilities,
    uint32_t owner, uint8_t role, ghostos_pci_inventory inventory, ghostos_driver_grant *out) {
    ghostos_driver_grant grant;
    ghostos_driver_grant_empty(&grant);
    if (role != GHOSTOS_DRIVER_ROLE_AHCI && role != GHOSTOS_DRIVER_ROLE_NVME &&
        role != GHOSTOS_DRIVER_ROLE_ETHERNET) { *out = grant; return GHOSTOS_GRANT_OK; }
    for (size_t i = 0; i < inventory.count; ++i) {
        const ghostos_pci_device *device = &inventory.devices[i];
        ghostos_driver_candidate candidate;
        ghostos_driver_selection selected = ghostos_driver_select(role, device,
            grant.manifest.count, grant.mapping_count, &candidate);
        if (selected == GHOSTOS_DRIVER_SKIP) continue;
        if (selected == GHOSTOS_DRIVER_CAPACITY) return GHOSTOS_GRANT_CAPACITY;
        ghostos_physical_range physical = candidate.physical;
        uint32_t dma_id = candidate.device;
        ghostos_capability_handle dma_cap;
        ghostos_capability_object dma_object = ghostos_capability_object_make(
            GHOSTOS_OBJECT_DMA_DEVICE, 0, 0, dma_id, 0);
        if (ghostos_capability_mint_root(capabilities, owner, dma_object,
                GHOSTOS_RIGHT_DMA_READ | GHOSTOS_RIGHT_DMA_WRITE | GHOSTOS_RIGHT_DELEGATE,
                &dma_cap) != GHOSTOS_CAP_OK) return GHOSTOS_GRANT_CAPABILITY;
        uint64_t mmio_cap = 0, virtual_address = 0;
        if (candidate.has_mmio) {
            ghostos_capability_range capability_range = {physical.start, physical.length};
            if (ghostos_capability_mint_mmio(capabilities, owner, capability_range,
                    GHOSTOS_RIGHT_MAP | GHOSTOS_RIGHT_READ | GHOSTOS_RIGHT_WRITE | GHOSTOS_RIGHT_DELEGATE,
                    &mmio_cap) != GHOSTOS_CAP_OK) return GHOSTOS_GRANT_CAPABILITY;
            virtual_address = candidate.virtual_address;
            grant.mappings[grant.mapping_count++] = (ghostos_driver_mmio_mapping){physical, virtual_address};
        }
        ghostos_driver_append_resource(&grant.manifest, &candidate, dma_cap, mmio_cap);
    }
    *out = grant;
    return GHOSTOS_GRANT_OK;
}
