#include "ghostos/micro_silo.h"

ghostos_silo_hardware_isolation ghostos_silo_software_only(void) {
    return (ghostos_silo_hardware_isolation){GHOSTOS_CONFIDENTIAL_CPU_NONE, false, false, false};
}

ghostos_silo_error ghostos_silo_hardware_protection(
    ghostos_silo_hardware_isolation hardware, ghostos_silo_memory_protection *protection) {
    if (!protection) return GHOSTOS_SILO_INVALID_RANGE;
    if (hardware.cxl_ide_available && !hardware.cxl_ide_enabled) return GHOSTOS_SILO_CXL_IDE_REQUIRED;
    if (hardware.confidential_cpu != GHOSTOS_CONFIDENTIAL_CPU_NONE && !hardware.memory_encryption_enabled)
        return GHOSTOS_SILO_MEMORY_ENCRYPTION_REQUIRED;
    *protection = hardware.cxl_ide_enabled && hardware.memory_encryption_enabled ?
        GHOSTOS_SILO_HARDWARE_ENCRYPTED : GHOSTOS_SILO_KERNEL_ISOLATED;
    return GHOSTOS_SILO_OK;
}

ghostos_silo_error ghostos_silo_memory_range_new(uint64_t start, uint64_t length,
    bool borrowed, ghostos_silo_memory_range *range) {
    if (!range || !length || UINT64_MAX - start < length) return GHOSTOS_SILO_INVALID_RANGE;
    *range = (ghostos_silo_memory_range){start, length, borrowed, true};
    return GHOSTOS_SILO_OK;
}

ghostos_silo_error ghostos_blind_micro_silo_init(ghostos_blind_micro_silo *silo,
    uint32_t address_space, size_t range_capacity, ghostos_silo_hardware_isolation hardware) {
    if (!silo || !address_space || !range_capacity || range_capacity > GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES)
        return GHOSTOS_SILO_INVALID_RANGE;
    ghostos_silo_memory_protection protection;
    ghostos_silo_error error = ghostos_silo_hardware_protection(hardware, &protection);
    if (error != GHOSTOS_SILO_OK) return error;
    *silo = (ghostos_blind_micro_silo){0};
    silo->address_space = address_space;
    silo->protection = protection;
    silo->range_capacity = range_capacity;
    return GHOSTOS_SILO_OK;
}

uint32_t ghostos_blind_micro_silo_address_space(const ghostos_blind_micro_silo *silo) {
    return silo ? silo->address_space : 0;
}

ghostos_silo_memory_protection ghostos_blind_micro_silo_protection(const ghostos_blind_micro_silo *silo) {
    return silo ? silo->protection : GHOSTOS_SILO_KERNEL_ISOLATED;
}

ghostos_silo_error ghostos_blind_micro_silo_authorize(ghostos_silo_object object,
    ghostos_silo_operation operation) {
    (void)operation;
    switch (object) {
        case GHOSTOS_SILO_TENANT_PROCESS:
        case GHOSTOS_SILO_TENANT_SYNFS:
        case GHOSTOS_SILO_TENANT_SOCKET:
            return GHOSTOS_SILO_OK;
        case GHOSTOS_SILO_HOST_PROCESS_TREE:
        case GHOSTOS_SILO_HOST_SYNFS_MOUNT:
        case GHOSTOS_SILO_HOST_NETWORK_SOCKET:
        case GHOSTOS_SILO_FEDERATION_CONTROL:
            return GHOSTOS_SILO_ACCESS_DENIED;
    }
    return GHOSTOS_SILO_ACCESS_DENIED;
}

static bool overlaps(ghostos_silo_memory_range left, ghostos_silo_memory_range right) {
    return left.start < right.start + right.length && right.start < left.start + left.length;
}

ghostos_silo_error ghostos_blind_micro_silo_map_memory(ghostos_blind_micro_silo *silo,
    ghostos_silo_memory_range range) {
    if (!silo || !range.length || UINT64_MAX - range.start < range.length ||
        !silo->range_capacity || silo->range_capacity > GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES)
        return GHOSTOS_SILO_INVALID_RANGE;
    for (size_t i = 0; i < silo->range_capacity; ++i)
        if (silo->ranges[i].occupied && overlaps(silo->ranges[i], range)) return GHOSTOS_SILO_RANGE_CONFLICT;
    for (size_t i = 0; i < silo->range_capacity; ++i) {
        if (!silo->ranges[i].occupied) {
            range.occupied = true;
            silo->ranges[i] = range;
            return GHOSTOS_SILO_OK;
        }
    }
    return GHOSTOS_SILO_CAPACITY;
}

size_t ghostos_blind_micro_silo_unmap_borrowed_memory(ghostos_blind_micro_silo *silo) {
    if (!silo || silo->range_capacity > GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES) return 0;
    size_t unmapped = 0;
    for (size_t i = 0; i < silo->range_capacity; ++i) {
        if (silo->ranges[i].occupied && silo->ranges[i].borrowed) {
            silo->ranges[i] = (ghostos_silo_memory_range){0};
            ++unmapped;
        }
    }
    return unmapped;
}

bool ghostos_blind_micro_silo_contains_memory(const ghostos_blind_micro_silo *silo,
    uint64_t address) {
    if (!silo || silo->range_capacity > GHOSTOS_MICRO_SILO_MAX_MEMORY_RANGES) return false;
    for (size_t i = 0; i < silo->range_capacity; ++i) {
        const ghostos_silo_memory_range *range = &silo->ranges[i];
        if (range->occupied && address >= range->start && address < range->start + range->length) return true;
    }
    return false;
}

_Static_assert(sizeof(ghostos_silo_memory_range) == 24, "silo range ABI");
_Static_assert(offsetof(ghostos_silo_memory_range, occupied) == 17, "silo occupancy ABI");
_Static_assert(sizeof(ghostos_silo_hardware_isolation) == 8, "silo hardware ABI");

int ghostos_silo_ranges_map(ghostos_silo_memory_range *ranges, size_t capacity,
    ghostos_silo_memory_range range, bool checked) {
    for (size_t i = 0; i < capacity; ++i) {
        if (!ranges[i].occupied) continue;
        if (checked && UINT64_MAX - range.start < range.length) return -1;
        if (ranges[i].start >= range.start + range.length) continue;
        if (checked && UINT64_MAX - ranges[i].start < ranges[i].length) return -1;
        if (range.start < ranges[i].start + ranges[i].length) return 1;
    }
    for (size_t i = 0; i < capacity; ++i) {
        if (ranges[i].occupied) continue;
        range.occupied = true;
        ranges[i] = range;
        return 0;
    }
    return 2;
}

size_t ghostos_silo_ranges_unmap_borrowed(ghostos_silo_memory_range *ranges, size_t capacity) {
    size_t count = 0;
    for (size_t i = 0; i < capacity; ++i) {
        if (ranges[i].occupied && ranges[i].borrowed) {
            ranges[i] = (ghostos_silo_memory_range){0};
            ++count;
        }
    }
    return count;
}

int ghostos_silo_ranges_contains(const ghostos_silo_memory_range *ranges,
    size_t capacity, uint64_t address, bool checked) {
    for (size_t i = 0; i < capacity; ++i) {
        if (!ranges[i].occupied || address < ranges[i].start) continue;
        if (checked && UINT64_MAX - ranges[i].start < ranges[i].length) return -1;
        if (address < ranges[i].start + ranges[i].length) return 1;
    }
    return 0;
}
