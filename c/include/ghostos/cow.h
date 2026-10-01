#ifndef GHOSTOS_COW_H
#define GHOSTOS_COW_H

#include "ghostos/frame_allocator.h"

#define GHOSTOS_MAX_COW_PAGES 4096

typedef enum {
    GHOSTOS_COW_OK = 0,
    GHOSTOS_COW_INVALID_RANGE,
    GHOSTOS_COW_NOT_TRACKED,
    GHOSTOS_COW_ALREADY_TRACKED,
    GHOSTOS_COW_CAPACITY,
    GHOSTOS_COW_INVALID_OWNER,
    GHOSTOS_COW_ALLOCATION,
    GHOSTOS_COW_COPY_FAILED,
    GHOSTOS_COW_RECLAIM_FAILED
} ghostos_cow_error;

typedef struct {
    uint64_t frame;
    uint32_t owner;
    uint32_t references;
} ghostos_cow_page_info;

typedef enum { GHOSTOS_COW_WRITE_EXCLUSIVE, GHOSTOS_COW_WRITE_COPIED } ghostos_cow_write_kind;
typedef struct {
    ghostos_cow_write_kind kind;
    uint64_t old_frame, new_frame;
} ghostos_cow_write_result;
typedef struct { uint64_t frame; uint32_t owner, references; bool used; } ghostos_cow_page;
typedef struct { ghostos_cow_page pages[GHOSTOS_MAX_COW_PAGES]; } ghostos_cow_manager;
typedef bool (*ghostos_cow_copy_page_fn)(void *context, uint64_t source, uint64_t destination);

void ghostos_cow_init(ghostos_cow_manager *manager);
ghostos_cow_error ghostos_cow_register(ghostos_cow_manager *manager, uint32_t owner, ghostos_physical_range backing);
ghostos_cow_error ghostos_cow_share(ghostos_cow_manager *manager, uint32_t owner, ghostos_physical_range backing);
ghostos_cow_error ghostos_cow_write_fault(ghostos_cow_manager *manager, uint32_t address_space, uint64_t frame, ghostos_early_frame_allocator *allocator, ghostos_cow_copy_page_fn copy_page, void *context, ghostos_cow_write_result *result);
ghostos_cow_error ghostos_cow_release(ghostos_cow_manager *manager, ghostos_physical_range backing, ghostos_early_frame_allocator *allocator, uint64_t *reclaimed);
bool ghostos_cow_info(const ghostos_cow_manager *manager, uint64_t frame, ghostos_cow_page_info *info);

#endif
