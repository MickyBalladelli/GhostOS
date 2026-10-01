#ifndef GHOSTOS_DMA_H
#define GHOSTOS_DMA_H

#include "ghostos/capability.h"

#define GHOSTOS_MAX_DMA_MAPPINGS 256
#define GHOSTOS_DMA_PAGE_SIZE UINT64_C(4096)

typedef enum {
    GHOSTOS_DMA_OK = 0,
    GHOSTOS_DMA_INVALID_RANGE,
    GHOSTOS_DMA_INVALID_PERMISSIONS,
    GHOSTOS_DMA_INVALID_CAPABILITY,
    GHOSTOS_DMA_ACCESS_DENIED,
    GHOSTOS_DMA_CAPACITY,
    GHOSTOS_DMA_IOVA_EXHAUSTED,
    GHOSTOS_DMA_IOMMU_REJECTED,
    GHOSTOS_DMA_MAPPING_NOT_FOUND
} ghostos_dma_error;

typedef enum {
    GHOSTOS_DMA_DEVICE_READ = 1u << 0,
    GHOSTOS_DMA_DEVICE_WRITE = 1u << 1
} ghostos_dma_permission;

typedef struct {
    uint64_t id;
    uint32_t device;
    uint64_t iova;
    ghostos_capability_range physical;
    uint8_t permissions;
} ghostos_dma_mapping;

typedef bool (*ghostos_iommu_map_fn)(void *context, uint32_t device, uint64_t iova,
    ghostos_capability_range physical, uint8_t permissions);
typedef bool (*ghostos_iommu_unmap_fn)(void *context, uint32_t device, uint64_t iova, uint64_t length);
typedef struct { ghostos_iommu_map_fn map; ghostos_iommu_unmap_fn unmap; void *context; } ghostos_iommu;

typedef struct {
    ghostos_dma_mapping mapping;
    uint32_t owner;
    ghostos_capability_handle device_authority, buffer_authority;
    bool occupied;
} ghostos_dma_record;

typedef struct {
    uint64_t next_id, next_iova;
    size_t capacity;
    ghostos_dma_record mappings[GHOSTOS_MAX_DMA_MAPPINGS];
} ghostos_dma_manager;

void ghostos_dma_init(ghostos_dma_manager *manager, size_t capacity);
ghostos_dma_error ghostos_dma_map(ghostos_dma_manager *manager,
    const ghostos_capability_space *capabilities, uint32_t caller,
    ghostos_capability_handle device_authority, ghostos_capability_handle buffer_authority,
    uint32_t device, uint64_t offset, uint64_t length, uint8_t permissions,
    ghostos_iommu iommu, ghostos_dma_mapping *out);
ghostos_dma_error ghostos_dma_unmap(ghostos_dma_manager *manager,
    const ghostos_capability_space *capabilities, uint32_t caller,
    ghostos_capability_handle device_authority, ghostos_capability_handle buffer_authority,
    uint64_t id, ghostos_iommu iommu, ghostos_dma_mapping *out);
bool ghostos_dma_mapping_get(const ghostos_dma_manager *manager, uint64_t id, ghostos_dma_mapping *out);

#endif
