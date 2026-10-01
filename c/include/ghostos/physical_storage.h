#ifndef GHOSTOS_PHYSICAL_STORAGE_H
#define GHOSTOS_PHYSICAL_STORAGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PHYSICAL_STORAGE_DEVICE_CAPACITY 64u

typedef struct {
    uint8_t bus, device, function;
    uint8_t class_code, subclass, programming_interface;
    uint8_t bar5_kind;
    uint64_t bar5_address;
} ghostos_storage_pci_device;

bool ghostos_physical_storage_missing_volume(
    const ghostos_storage_pci_device *devices, size_t count, bool mounted);
size_t ghostos_physical_storage_next_ahci(
    const ghostos_storage_pci_device *devices, size_t count, size_t start);

#endif
