#include "ghostos/pci.h"

#if defined(__x86_64__)
static void write_config_address(uint32_t address) {
    __asm__ volatile("outl %0, %w1" : : "a"(address), "Nd"(UINT16_C(0x0cf8)) : "memory");
}

static uint32_t read_config(ghostos_pci_device_info address, uint8_t offset) {
    uint32_t key = UINT32_C(0x80000000) | (uint32_t)address.bus << 16 |
        (uint32_t)address.device << 11 | (uint32_t)address.function << 8 | (offset & 0xfcu);
    write_config_address(key);
    uint32_t value;
    __asm__ volatile("inl %w1, %0" : "=a"(value) : "Nd"(UINT16_C(0x0cfc)) : "memory");
    return value;
}

static bool read_device(ghostos_pci_device_info *device) {
    uint32_t identity = read_config(*device, 0x00);
    device->vendor_id = (uint16_t)identity;
    if (device->vendor_id == GHOSTOS_PCI_INVALID_VENDOR) return false;
    device->device_id = (uint16_t)(identity >> 16);

    uint32_t class_register = read_config(*device, 0x08);
    device->revision = (uint8_t)class_register;
    device->programming_interface = (uint8_t)(class_register >> 8);
    device->subclass = (uint8_t)(class_register >> 16);
    device->class_code = (uint8_t)(class_register >> 24);
    device->header_type = (uint8_t)(read_config(*device, 0x0c) >> 16);

    if ((device->header_type & 0x7f) == 0) {
        for (size_t index = 0; index < 6;) {
            uint32_t low = read_config(*device, (uint8_t)(0x10 + index * 4));
            if (!low) { ++index; continue; }
            ghostos_pci_bar_info *bar = &device->bars[index];
            if (low & 1) {
                *bar = (ghostos_pci_bar_info){GHOSTOS_PCI_BAR_IO, low & ~UINT32_C(3), false};
                ++index;
                continue;
            }
            bool prefetchable = (low & 8) != 0;
            if ((low & 6) == 4 && index + 1 < 6) {
                uint32_t high = read_config(*device, (uint8_t)(0x14 + index * 4));
                *bar = (ghostos_pci_bar_info){GHOSTOS_PCI_BAR_MEMORY64,
                    ((uint64_t)high << 32) | (low & ~UINT32_C(0xf)), prefetchable};
                index += 2;
            } else {
                *bar = (ghostos_pci_bar_info){GHOSTOS_PCI_BAR_MEMORY32,
                    low & ~UINT32_C(0xf), prefetchable};
                ++index;
            }
        }
    }

    uint32_t interrupt = read_config(*device, 0x3c);
    device->interrupt_line = (uint8_t)interrupt;
    device->interrupt_pin = (uint8_t)(interrupt >> 8);
    return true;
}

void ghostos_pci_enumerate_x86(ghostos_pci_inventory_info *inventory) {
    if (!inventory) return;
    *inventory = (ghostos_pci_inventory_info){0};
    for (uint16_t bus = 0; bus <= UINT8_MAX; ++bus) {
        for (uint8_t device = 0; device < 32; ++device) {
            ghostos_pci_device_info address = {.bus = (uint8_t)bus, .device = device};
            if ((uint16_t)read_config(address, 0) == GHOSTOS_PCI_INVALID_VENDOR) continue;
            uint8_t header = (uint8_t)(read_config(address, 0x0c) >> 16);
            uint8_t functions = (header & 0x80) ? 8 : 1;
            for (uint8_t function = 0; function < functions; ++function) {
                ghostos_pci_device_info current = {
                    .bus = (uint8_t)bus, .device = device, .function = function};
                if (read_device(&current)) {
                    if (inventory->count < GHOSTOS_PCI_DEVICE_CAPACITY) {
                        inventory->devices[inventory->count++] = current;
                    }
                }
            }
        }
    }
}
#else
void ghostos_pci_enumerate_x86(ghostos_pci_inventory_info *inventory) {
    if (inventory) *inventory = (ghostos_pci_inventory_info){0};
}
#endif
