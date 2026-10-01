#include "ghostos/driver_capabilities.h"

static bool add_u64(uint64_t a, uint64_t b, uint64_t *out) {
    if (UINT64_MAX - a < b) return false;
    *out = a + b;
    return true;
}

void ghostos_driver_grant_empty(ghostos_driver_grant *grant) {
    *grant = (ghostos_driver_grant){0};
    grant->manifest.magic = GHOSTOS_SERVICE_RESOURCE_MAGIC;
    grant->manifest.version = GHOSTOS_SERVICE_RESOURCE_VERSION;
}

bool ghostos_service_mmio_overlaps_image(uint64_t virtual_address, uint64_t length) {
    uint64_t end;
    uint64_t image_start = GHOSTOS_USER_SPACE_START;
    uint64_t image_end = image_start +
        (GHOSTOS_SERVICE_CODE_PAGE_COUNT + GHOSTOS_SERVICE_STACK_PAGE_COUNT) * GHOSTOS_PAGE_SIZE;
    if (!length || !add_u64(virtual_address, length, &end)) return true;
    return virtual_address < image_end && end > image_start;
}

static bool matches_ethernet(const ghostos_pci_device *device, uint64_t *bar_address, uint64_t *bar_length,
    bool *has_mmio) {
    if (device->class_code != 0x02 || device->subclass != 0x00) return false;
    bool e1000 = device->vendor_id == 0x8086 &&
        (device->device_id == 0x100e || device->device_id == 0x100f || device->device_id == 0x1010 ||
         device->device_id == 0x107c || device->device_id == 0x10d3 || device->device_id == 0x150c ||
         device->device_id == 0x1533 || device->device_id == 0x1539 || device->device_id == 0x157b);
    bool realtek = device->vendor_id == 0x10ec &&
        (device->device_id == 0x8161 || device->device_id == 0x8168 || device->device_id == 0x8169);
    bool virtio = device->vendor_id == 0x1af4 && device->device_id == 0x1000;
    if (!e1000 && !realtek && !virtio) return false;
    for (size_t i = 0; i < 6; ++i) {
        const ghostos_pci_bar *bar = &device->bars[i];
        if (virtio) {
            if (bar->kind == GHOSTOS_PCI_BAR_IO) {
                *has_mmio = false;
                return true;
            }
        } else if (bar->kind == GHOSTOS_PCI_BAR_MEMORY32 || bar->kind == GHOSTOS_PCI_BAR_MEMORY64) {
            *bar_address = bar->address;
            *bar_length = e1000 ? UINT64_C(0x4000) : UINT64_C(0x1000);
            *has_mmio = true;
            return true;
        }
    }
    return false;
}

static bool matching_bar(uint8_t role, const ghostos_pci_device *device, uint32_t *resource_kind,
    uint64_t *address, uint64_t *length, bool *has_mmio) {
    *has_mmio = true;
    if (role == GHOSTOS_DRIVER_ROLE_AHCI && device->class_code == 0x01 && device->subclass == 0x06 &&
            device->programming_interface == 0x01) {
        *resource_kind = 1;
        if (device->bars[5].kind == GHOSTOS_PCI_BAR_MEMORY32 || device->bars[5].kind == GHOSTOS_PCI_BAR_MEMORY64) {
            *address = device->bars[5].address; *length = UINT64_C(0x2000);
        } else *has_mmio = false;
        return true;
    }
    if (role == GHOSTOS_DRIVER_ROLE_NVME && device->class_code == 0x01 && device->subclass == 0x08 &&
            device->programming_interface == 0x02) {
        *resource_kind = 2;
        if (device->bars[0].kind == GHOSTOS_PCI_BAR_MEMORY32 || device->bars[0].kind == GHOSTOS_PCI_BAR_MEMORY64) {
            *address = device->bars[0].address; *length = UINT64_C(0x1000);
        } else *has_mmio = false;
        return true;
    }
    if (role == GHOSTOS_DRIVER_ROLE_ETHERNET) {
        *resource_kind = 3;
        return matches_ethernet(device, address, length, has_mmio);
    }
    return false;
}

static uint32_t device_id(const ghostos_pci_device *device) {
    return 1u + ((uint32_t)device->bus << 16) + ((uint32_t)device->device << 8) + device->function;
}

ghostos_grant_error ghostos_driver_grant_resources(ghostos_capability_space *capabilities,
    uint32_t owner, uint8_t role, ghostos_pci_inventory inventory, ghostos_driver_grant *out) {
    ghostos_driver_grant grant;
    ghostos_driver_grant_empty(&grant);
    if (role != GHOSTOS_DRIVER_ROLE_AHCI && role != GHOSTOS_DRIVER_ROLE_NVME &&
        role != GHOSTOS_DRIVER_ROLE_ETHERNET) { *out = grant; return GHOSTOS_GRANT_OK; }
    for (size_t i = 0; i < inventory.count; ++i) {
        const ghostos_pci_device *device = &inventory.devices[i];
        uint32_t kind;
        uint64_t address = 0, length = 0;
        bool has_mmio;
        if (!matching_bar(role, device, &kind, &address, &length, &has_mmio)) continue;
        if (grant.manifest.count == GHOSTOS_MAX_DRIVER_RESOURCES) return GHOSTOS_GRANT_CAPACITY;
        ghostos_physical_range physical = {address, length};
        if (has_mmio && (!length || address % GHOSTOS_PAGE_SIZE || length % GHOSTOS_PAGE_SIZE ||
                !add_u64(address, length, &(uint64_t){0}))) continue;

        uint32_t dma_id = device_id(device);
        ghostos_capability_handle dma_cap;
        ghostos_capability_object dma_object = ghostos_capability_object_make(
            GHOSTOS_OBJECT_DMA_DEVICE, 0, 0, dma_id, 0);
        if (ghostos_capability_mint_root(capabilities, owner, dma_object,
                GHOSTOS_RIGHT_DMA_READ | GHOSTOS_RIGHT_DMA_WRITE | GHOSTOS_RIGHT_DELEGATE,
                &dma_cap) != GHOSTOS_CAP_OK) return GHOSTOS_GRANT_CAPABILITY;
        uint64_t mmio_cap = 0, virtual_address = 0, mapped_length = 0;
        if (has_mmio) {
            ghostos_capability_range capability_range = {physical.start, physical.length};
            if (ghostos_capability_mint_mmio(capabilities, owner, capability_range,
                    GHOSTOS_RIGHT_MAP | GHOSTOS_RIGHT_READ | GHOSTOS_RIGHT_WRITE | GHOSTOS_RIGHT_DELEGATE,
                    &mmio_cap) != GHOSTOS_CAP_OK) return GHOSTOS_GRANT_CAPABILITY;
            virtual_address = GHOSTOS_SERVICE_MMIO_BASE + grant.mapping_count * GHOSTOS_SERVICE_MMIO_STRIDE;
            mapped_length = length;
            grant.mappings[grant.mapping_count++] = (ghostos_driver_mmio_mapping){physical, virtual_address};
        }
        size_t index = grant.manifest.count++;
        grant.manifest.resources[index] = (ghostos_service_resource){
            kind, mmio_cap, dma_cap, has_mmio ? address : 0, virtual_address, mapped_length
        };
    }
    *out = grant;
    return GHOSTOS_GRANT_OK;
}
