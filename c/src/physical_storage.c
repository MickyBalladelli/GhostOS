#include "ghostos/physical_storage.h"

bool ghostos_physical_storage_missing_volume(
    const ghostos_storage_pci_device *devices, size_t count, bool mounted) {
    if (mounted || !devices) return false;
    if (count > GHOSTOS_PHYSICAL_STORAGE_DEVICE_CAPACITY) {
        count = GHOSTOS_PHYSICAL_STORAGE_DEVICE_CAPACITY;
    }
    for (size_t index = 0; index < count; ++index) {
        const ghostos_storage_pci_device *device = &devices[index];
        if (device->class_code == 0x01 && device->subclass == 0x06 &&
            device->programming_interface == 0x01) {
            return true;
        }
    }
    return false;
}

size_t ghostos_physical_storage_next_ahci(
    const ghostos_storage_pci_device *devices, size_t count, size_t start) {
    if (!devices) return count;
    if (count > GHOSTOS_PHYSICAL_STORAGE_DEVICE_CAPACITY) {
        count = GHOSTOS_PHYSICAL_STORAGE_DEVICE_CAPACITY;
    }
    for (size_t index = start; index < count; ++index) {
        const ghostos_storage_pci_device *device = &devices[index];
        bool ahci = device->class_code == 0x01 && device->subclass == 0x06 &&
            device->programming_interface == 0x01;
        bool memory_bar = device->bar5_kind == 2 || device->bar5_kind == 3;
        if (ahci && memory_bar && device->bar5_address) return index;
    }
    return count;
}
