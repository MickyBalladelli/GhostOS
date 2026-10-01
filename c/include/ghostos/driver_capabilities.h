#ifndef GHOSTOS_DRIVER_CAPABILITIES_H
#define GHOSTOS_DRIVER_CAPABILITIES_H

#include "ghostos/address_space.h"
#include "ghostos/capability.h"

#define GHOSTOS_MAX_DRIVER_RESOURCES 16
#define GHOSTOS_SERVICE_RESOURCE_MAGIC UINT64_C(0x53594e4f44525653)
#define GHOSTOS_SERVICE_RESOURCE_VERSION 1u
#define GHOSTOS_SERVICE_RESOURCE_STATE_OFFSET UINT64_C(8)
#define GHOSTOS_SERVICE_MMIO_STRIDE UINT64_C(0x10000)
#define GHOSTOS_SERVICE_CODE_PAGE_COUNT 19u
#define GHOSTOS_SERVICE_STACK_PAGE_COUNT 16u
#define GHOSTOS_DRIVER_ROLE_AHCI 11u
#define GHOSTOS_DRIVER_ROLE_NVME 12u
#define GHOSTOS_DRIVER_ROLE_ETHERNET 13u

#define GHOSTOS_SERVICE_MMIO_BASE \
    ((((GHOSTOS_USER_SPACE_START + \
      (GHOSTOS_SERVICE_CODE_PAGE_COUNT + GHOSTOS_SERVICE_STACK_PAGE_COUNT) * GHOSTOS_PAGE_SIZE) + \
      GHOSTOS_SERVICE_MMIO_STRIDE - 1) / GHOSTOS_SERVICE_MMIO_STRIDE) * GHOSTOS_SERVICE_MMIO_STRIDE)

typedef enum {
    GHOSTOS_PCI_BAR_UNUSED,
    GHOSTOS_PCI_BAR_IO,
    GHOSTOS_PCI_BAR_MEMORY32,
    GHOSTOS_PCI_BAR_MEMORY64
} ghostos_pci_bar_kind;

typedef struct { ghostos_pci_bar_kind kind; uint64_t address; } ghostos_pci_bar;
typedef struct {
    uint8_t bus, device, function;
    uint16_t vendor_id, device_id;
    uint8_t programming_interface, subclass, class_code;
    ghostos_pci_bar bars[6];
} ghostos_pci_device;
typedef struct { const ghostos_pci_device *devices; size_t count; } ghostos_pci_inventory;

typedef struct {
    uint32_t kind;
    uint64_t capability, dma_capability;
    uint64_t physical, virtual_address, length;
} ghostos_service_resource;

typedef struct {
    uint64_t magic;
    uint32_t version, count;
    ghostos_service_resource resources[GHOSTOS_MAX_DRIVER_RESOURCES];
} ghostos_service_resource_manifest;

typedef struct { ghostos_physical_range physical; uint64_t virtual_address; } ghostos_driver_mmio_mapping;
typedef struct {
    ghostos_service_resource_manifest manifest;
    ghostos_driver_mmio_mapping mappings[GHOSTOS_MAX_DRIVER_RESOURCES];
    size_t mapping_count;
} ghostos_driver_grant;

typedef enum { GHOSTOS_GRANT_OK, GHOSTOS_GRANT_CAPABILITY, GHOSTOS_GRANT_CAPACITY } ghostos_grant_error;

void ghostos_driver_grant_empty(ghostos_driver_grant *grant);
bool ghostos_service_mmio_overlaps_image(uint64_t virtual_address, uint64_t length);
ghostos_grant_error ghostos_driver_grant_resources(ghostos_capability_space *capabilities,
    uint32_t owner, uint8_t role, ghostos_pci_inventory inventory, ghostos_driver_grant *out);

#endif
