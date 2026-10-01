#ifndef GHOSTOS_PCI_H
#define GHOSTOS_PCI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PCI_INVALID_VENDOR UINT16_C(0xffff)
#define GHOSTOS_PCI_DEVICE_CAPACITY 64u

typedef enum {
    GHOSTOS_PCI_BAR_UNUSED = 0,
    GHOSTOS_PCI_BAR_IO = 1,
    GHOSTOS_PCI_BAR_MEMORY32 = 2,
    GHOSTOS_PCI_BAR_MEMORY64 = 3
} ghostos_pci_bar_kind;

typedef struct {
    ghostos_pci_bar_kind kind;
    uint64_t address;
    bool prefetchable;
} ghostos_pci_bar_info;

typedef struct {
    uint8_t bus, device, function;
    uint16_t vendor_id, device_id;
    uint8_t revision, programming_interface, subclass, class_code, header_type;
    ghostos_pci_bar_info bars[6];
    uint8_t interrupt_line, interrupt_pin;
} ghostos_pci_device_info;

typedef void (*ghostos_pci_visit_fn)(void *context, const ghostos_pci_device_info *device);

size_t ghostos_pci_enumerate_x86(ghostos_pci_visit_fn visit, void *context);

#endif
