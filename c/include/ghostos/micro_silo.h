#ifndef GHOSTOS_MICRO_SILO_H
#define GHOSTOS_MICRO_SILO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES 16u

typedef enum {
    GHOSTOS_SILO_TENANT_PROCESS = 0,
    GHOSTOS_SILO_TENANT_SYNFS = 1,
    GHOSTOS_SILO_TENANT_SOCKET = 2,
    GHOSTOS_SILO_HOST_PROCESS_TREE = 3,
    GHOSTOS_SILO_HOST_SYNFS_MOUNT = 4,
    GHOSTOS_SILO_HOST_NETWORK_SOCKET = 5,
    GHOSTOS_SILO_FEDERATION_CONTROL = 6
} ghostos_silo_object;

typedef enum {
    GHOSTOS_SILO_INSPECT = 0,
    GHOSTOS_SILO_READ = 1,
    GHOSTOS_SILO_WRITE = 2,
    GHOSTOS_SILO_CONNECT = 3
} ghostos_silo_operation;

typedef enum {
    GHOSTOS_CONFIDENTIAL_CPU_NONE = 0,
    GHOSTOS_CONFIDENTIAL_CPU_AMD_SEV = 1,
    GHOSTOS_CONFIDENTIAL_CPU_INTEL_TDX = 2,
    GHOSTOS_CONFIDENTIAL_CPU_ARM_CCA = 3
} ghostos_confidential_cpu;

typedef struct {
    ghostos_confidential_cpu confidential_cpu;
    bool cxl_ide_available;
    bool cxl_ide_enabled;
    bool memory_encryption_enabled;
} ghostos_silo_hardware_isolation;

typedef enum {
    GHOSTOS_SILO_KERNEL_ISOLATED = 0,
    GHOSTOS_SILO_HARDWARE_ENCRYPTED = 1
} ghostos_silo_memory_protection;

typedef enum {
    GHOSTOS_SILO_OK = 0,
    GHOSTOS_SILO_ACCESS_DENIED,
    GHOSTOS_SILO_CAPACITY,
    GHOSTOS_SILO_CXL_IDE_REQUIRED,
    GHOSTOS_SILO_INVALID_RANGE,
    GHOSTOS_SILO_MEMORY_ENCRYPTION_REQUIRED,
    GHOSTOS_SILO_RANGE_CONFLICT
} ghostos_silo_error;

typedef struct {
    uint64_t start;
    uint64_t length;
    bool borrowed;
    bool occupied;
} ghostos_silo_memory_range;

typedef struct {
    uint32_t address_space;
    ghostos_silo_memory_protection protection;
    size_t range_capacity;
    ghostos_silo_memory_range ranges[GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES];
} ghostos_blind_micro_silo;

ghostos_silo_hardware_isolation ghostos_silo_software_only(void);
ghostos_silo_error ghostos_silo_hardware_protection(
    ghostos_silo_hardware_isolation hardware, ghostos_silo_memory_protection *protection);
ghostos_silo_error ghostos_silo_memory_range_new(uint64_t start, uint64_t length,
    bool borrowed, ghostos_silo_memory_range *range);
ghostos_silo_error ghostos_blind_micro_silo_init(ghostos_blind_micro_silo *silo,
    uint32_t address_space, size_t range_capacity, ghostos_silo_hardware_isolation hardware);
uint32_t ghostos_blind_micro_silo_address_space(const ghostos_blind_micro_silo *silo);
ghostos_silo_memory_protection ghostos_blind_micro_silo_protection(const ghostos_blind_micro_silo *silo);
ghostos_silo_error ghostos_blind_micro_silo_authorize(ghostos_silo_object object,
    ghostos_silo_operation operation);
ghostos_silo_error ghostos_blind_micro_silo_map_memory(ghostos_blind_micro_silo *silo,
    ghostos_silo_memory_range range);
size_t ghostos_blind_micro_silo_unmap_borrowed_memory(ghostos_blind_micro_silo *silo);
bool ghostos_blind_micro_silo_contains_memory(const ghostos_blind_micro_silo *silo,
    uint64_t address);

#endif
