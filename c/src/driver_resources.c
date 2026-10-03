#include "ghostos/driver_capabilities.h"

static bool add_u64(uint64_t a, uint64_t b, uint64_t *out) {
    if (UINT64_MAX - a < b) return false;
    *out = a + b;
    return true;
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


_Static_assert(GHOSTOS_SERVICE_MMIO_BASE == UINT64_C(0x0000008000030000), "driver MMIO base");
_Static_assert(sizeof(ghostos_pci_device) == 112, "driver PCI ABI");
_Static_assert(offsetof(ghostos_pci_device, bars) == 16, "driver BAR ABI");
_Static_assert(sizeof(ghostos_driver_candidate) == 40, "driver selection ABI");
_Static_assert(sizeof(ghostos_service_resource) == 48, "driver resource ABI");
_Static_assert(sizeof(ghostos_service_resource_manifest) == 784, "driver manifest ABI");

ghostos_driver_selection ghostos_driver_select(uint8_t role,
    const ghostos_pci_device *device, uint32_t resource_count, size_t mapping_count,
    ghostos_driver_candidate *candidate) {
    uint32_t kind;
    uint64_t address = 0, length = 0;
    bool has_mmio;
    if (!matching_bar(role, device, &kind, &address, &length, &has_mmio)) return GHOSTOS_DRIVER_SKIP;
    if (resource_count == GHOSTOS_MAX_DRIVER_RESOURCES) return GHOSTOS_DRIVER_CAPACITY;
    if (has_mmio && (!length || address % GHOSTOS_PAGE_SIZE || length % GHOSTOS_PAGE_SIZE ||
        !add_u64(address, length, &(uint64_t){0}))) return GHOSTOS_DRIVER_SKIP;
    *candidate = (ghostos_driver_candidate){kind, device_id(device), {address, length},
        has_mmio ? GHOSTOS_SERVICE_MMIO_BASE + mapping_count * GHOSTOS_SERVICE_MMIO_STRIDE : 0,
        has_mmio};
    return GHOSTOS_DRIVER_READY;
}

void ghostos_driver_append_resource(ghostos_service_resource_manifest *manifest,
    const ghostos_driver_candidate *candidate, uint64_t dma_capability, uint64_t mmio_capability) {
    manifest->resources[manifest->count++] = (ghostos_service_resource){
        candidate->kind, mmio_capability, dma_capability,
        candidate->has_mmio ? candidate->physical.start : 0,
        candidate->virtual_address, candidate->has_mmio ? candidate->physical.length : 0};
}
