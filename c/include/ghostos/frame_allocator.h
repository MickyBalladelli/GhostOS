#ifndef GHOSTOS_FRAME_ALLOCATOR_H
#define GHOSTOS_FRAME_ALLOCATOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_FRAME_SIZE UINT64_C(4096)
#define GHOSTOS_MAX_MEMORY_REGIONS 128
#define GHOSTOS_MAX_OWNED_FRAME_RANGES 256

typedef enum { GHOSTOS_MEMORY_USABLE = 1, GHOSTOS_MEMORY_RESERVED = 2,
    GHOSTOS_MEMORY_ACPI_RECLAIMABLE = 3, GHOSTOS_MEMORY_ACPI_NONVOLATILE = 4,
    GHOSTOS_MEMORY_BOOTLOADER = 5, GHOSTOS_MEMORY_KERNEL = 6,
    GHOSTOS_MEMORY_FRAMEBUFFER = 7 } ghostos_memory_kind;
typedef struct { uint64_t start, length; uint32_t kind, attributes; } ghostos_memory_region;
typedef struct { uint64_t start, length; } ghostos_physical_range;
typedef enum { GHOSTOS_ALLOC_OK, GHOSTOS_ALLOC_EXHAUSTED } ghostos_allocation_error;
typedef enum { GHOSTOS_RECLAIM_OK, GHOSTOS_RECLAIM_INVALID_RANGE, GHOSTOS_RECLAIM_NOT_OWNED,
    GHOSTOS_RECLAIM_CAPACITY, GHOSTOS_RECLAIM_ACCESS_DENIED } ghostos_reclaim_error;
typedef enum { GHOSTOS_QUOTA_ALLOWED, GHOSTOS_QUOTA_DECISION_THROTTLED, GHOSTOS_QUOTA_DECISION_REJECTED } ghostos_quota_decision;
typedef enum { GHOSTOS_QUOTA_OK, GHOSTOS_QUOTA_INVALID_CAPABILITY, GHOSTOS_QUOTA_EXHAUSTED,
    GHOSTOS_QUOTA_THROTTLED, GHOSTOS_QUOTA_REJECTED } ghostos_quota_error;
typedef struct { uint64_t start, length; bool used; } ghostos_free_frame_range;
typedef struct { uint64_t start, length; uint32_t owner; bool used; } ghostos_owned_frame_range;
typedef struct {
    ghostos_free_frame_range free_ranges[GHOSTOS_MAX_MEMORY_REGIONS];
    size_t free_count;
    ghostos_owned_frame_range owned_ranges[GHOSTOS_MAX_OWNED_FRAME_RANGES];
    size_t owned_count;
} ghostos_early_frame_allocator;

typedef struct {
    void *context;
    ghostos_quota_decision (*consume)(void *context, uint32_t owner, uint64_t now_us, uint64_t bytes, uint64_t *retry_after_us);
    void (*refund)(void *context, uint32_t owner, uint64_t bytes);
    bool (*release)(void *context, uint32_t owner, uint64_t bytes);
    bool (*authorize)(void *context, uint32_t caller, uint64_t authority);
} ghostos_frame_quota;

void ghostos_frame_allocator_init(ghostos_early_frame_allocator *allocator, const ghostos_memory_region *regions, size_t count);
ghostos_allocation_error ghostos_frame_allocate(ghostos_early_frame_allocator *allocator, uint64_t *frame);
ghostos_allocation_error ghostos_frame_allocate_for(ghostos_early_frame_allocator *allocator, uint32_t owner, uint64_t *frame);
ghostos_allocation_error ghostos_frame_allocate_range(ghostos_early_frame_allocator *allocator, uint32_t owner, size_t frame_count, ghostos_physical_range *range);
ghostos_quota_error ghostos_frame_allocate_quota(ghostos_early_frame_allocator *allocator, uint64_t now_us, const ghostos_frame_quota *quota, uint64_t *frame, uint64_t *retry_after_us);
ghostos_quota_error ghostos_frame_allocate_capability(ghostos_early_frame_allocator *allocator, uint32_t caller, uint64_t authority, uint64_t now_us, const ghostos_frame_quota *quota, uint64_t *frame, uint64_t *retry_after_us);
ghostos_reclaim_error ghostos_frame_reclaim(ghostos_early_frame_allocator *allocator, uint32_t owner, uint64_t frame);
ghostos_reclaim_error ghostos_frame_reclaim_range(ghostos_early_frame_allocator *allocator, uint32_t owner, ghostos_physical_range range);
ghostos_reclaim_error ghostos_frame_reclaim_quota(ghostos_early_frame_allocator *allocator, uint32_t owner, ghostos_physical_range range, const ghostos_frame_quota *quota);
ghostos_reclaim_error ghostos_frame_reclaim_capability(ghostos_early_frame_allocator *allocator, uint32_t caller, uint64_t authority, ghostos_physical_range range, const ghostos_frame_quota *quota);
bool ghostos_frame_owner_of(const ghostos_early_frame_allocator *allocator, uint64_t frame, uint32_t *owner);
uint64_t ghostos_frame_available(const ghostos_early_frame_allocator *allocator);
uint64_t ghostos_frame_owned(const ghostos_early_frame_allocator *allocator, uint32_t owner);

#endif
