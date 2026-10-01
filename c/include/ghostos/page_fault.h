#ifndef GHOSTOS_PAGE_FAULT_H
#define GHOSTOS_PAGE_FAULT_H

#include "ghostos/address_space.h"
#include "ghostos/cow.h"

typedef struct {
    uint64_t virtual_address;
    ghostos_memory_access access;
    bool user;
    bool present;
    bool reserved_bit;
    bool instruction_fetch;
} ghostos_page_fault;

typedef bool (*ghostos_page_fault_handler)(const ghostos_page_fault *fault, void *context);
typedef bool (*ghostos_page_fault_map_stack_fn)(void *context, uint32_t address_space, uint64_t page);

typedef enum {
    GHOSTOS_PAGE_FAULT_HANDLER_OK = 0,
    GHOSTOS_PAGE_FAULT_HANDLER_ALREADY_INSTALLED
} ghostos_page_fault_handler_error;

typedef enum {
    GHOSTOS_PAGE_FAULT_DISPATCH_OK = 0,
    GHOSTOS_PAGE_FAULT_INVALID_CAPABILITY,
    GHOSTOS_PAGE_FAULT_RATE_LIMITED,
    GHOSTOS_PAGE_FAULT_REJECTED
} ghostos_page_fault_dispatch_result;

typedef enum {
    GHOSTOS_STACK_FAULT_OK = 0,
    GHOSTOS_STACK_FAULT_INVALID,
    GHOSTOS_STACK_FAULT_ADDRESS_SPACE,
    GHOSTOS_STACK_FAULT_GROWTH,
    GHOSTOS_STACK_FAULT_MAPPING_FAILED
} ghostos_stack_fault_result;

typedef enum {
    GHOSTOS_COW_FAULT_OK = 0,
    GHOSTOS_COW_FAULT_INVALID,
    GHOSTOS_COW_FAULT_ADDRESS_SPACE,
    GHOSTOS_COW_FAULT_COW
} ghostos_cow_fault_result_kind;

typedef struct {
    bool copied;
    uint64_t old_frame;
    uint64_t new_frame;
} ghostos_cow_fault_result;

typedef enum { GHOSTOS_PAGE_FAULT_QUOTA_ALLOWED, GHOSTOS_PAGE_FAULT_QUOTA_THROTTLED,
    GHOSTOS_PAGE_FAULT_QUOTA_REJECTED, GHOSTOS_PAGE_FAULT_QUOTA_INVALID } ghostos_page_fault_quota_result;
typedef ghostos_page_fault_quota_result (*ghostos_page_fault_consume_fn)(
    void *context, uint32_t caller, uint64_t authority, uint64_t now_us, uint64_t *retry_after_us);
typedef void (*ghostos_page_fault_refund_fn)(void *context, uint32_t caller, uint64_t authority);

ghostos_page_fault ghostos_page_fault_from_x86_error(uint64_t virtual_address, uint64_t error);
uint64_t ghostos_page_fault_page_address(ghostos_page_fault fault);
ghostos_page_fault_handler_error ghostos_page_fault_install_handler(
    ghostos_page_fault_handler handler, void *context);
bool ghostos_page_fault_dispatch(ghostos_page_fault fault);
ghostos_page_fault_dispatch_result ghostos_page_fault_dispatch_for(
    uint32_t caller, uint64_t authority, uint64_t now_us, ghostos_page_fault fault,
    ghostos_page_fault_consume_fn consume, ghostos_page_fault_refund_fn refund,
    void *quota_context, uint64_t *retry_after_us, bool *handled);
ghostos_stack_fault_result ghostos_page_fault_resolve_stack(
    ghostos_address_space *address_space, ghostos_page_fault fault,
    ghostos_page_fault_map_stack_fn map_page, void *context, ghostos_stack_growth *growth);
ghostos_cow_fault_result_kind ghostos_page_fault_resolve_cow(
    ghostos_address_space *address_space, ghostos_cow_manager *cow,
    ghostos_early_frame_allocator *allocator, ghostos_cow_copy_page_fn copy_page,
    void *context, uint32_t owner, ghostos_page_fault fault,
    ghostos_cow_fault_result *result);

#endif
