#include "ghostos/page_fault.h"

#include <stdatomic.h>

static atomic_flag handler_lock = ATOMIC_FLAG_INIT;
static atomic_bool handler_installed;
static ghostos_page_fault_handler installed_handler;
static void *installed_context;

ghostos_page_fault ghostos_page_fault_from_x86_error(uint64_t virtual_address, uint64_t error) {
    return (ghostos_page_fault){virtual_address,
        (error & (UINT64_C(1) << 1)) ? GHOSTOS_ACCESS_WRITE : GHOSTOS_ACCESS_READ,
        (error & (UINT64_C(1) << 2)) != 0, (error & 1) != 0,
        (error & (UINT64_C(1) << 3)) != 0, (error & (UINT64_C(1) << 4)) != 0};
}

uint64_t ghostos_page_fault_page_address(ghostos_page_fault fault) {
    return fault.virtual_address & ~(GHOSTOS_PAGE_SIZE - 1);
}

ghostos_page_fault_handler_error ghostos_page_fault_install_handler(
    ghostos_page_fault_handler handler, void *context) {
    if (!handler) return GHOSTOS_PAGE_FAULT_HANDLER_ALREADY_INSTALLED;
    while (atomic_flag_test_and_set_explicit(&handler_lock, memory_order_acquire)) { }
    if (atomic_load_explicit(&handler_installed, memory_order_relaxed)) {
        atomic_flag_clear_explicit(&handler_lock, memory_order_release);
        return GHOSTOS_PAGE_FAULT_HANDLER_ALREADY_INSTALLED;
    }
    installed_handler = handler;
    installed_context = context;
    atomic_store_explicit(&handler_installed, true, memory_order_release);
    atomic_flag_clear_explicit(&handler_lock, memory_order_release);
    return GHOSTOS_PAGE_FAULT_HANDLER_OK;
}

bool ghostos_page_fault_dispatch(ghostos_page_fault fault) {
    if (!atomic_load_explicit(&handler_installed, memory_order_acquire)) return false;
    ghostos_page_fault_handler handler = installed_handler;
    return handler && handler(&fault, installed_context);
}

ghostos_page_fault_dispatch_result ghostos_page_fault_dispatch_for(
    uint32_t caller, uint64_t authority, uint64_t now_us, ghostos_page_fault fault,
    ghostos_page_fault_consume_fn consume, ghostos_page_fault_refund_fn refund,
    void *quota_context, uint64_t *retry_after_us, bool *handled) {
    if (retry_after_us) *retry_after_us = 0;
    if (handled) *handled = false;
    if (!consume) return GHOSTOS_PAGE_FAULT_INVALID_CAPABILITY;
    uint64_t retry = 0;
    ghostos_page_fault_quota_result decision = consume(quota_context, caller, authority, now_us, &retry);
    if (decision == GHOSTOS_PAGE_FAULT_QUOTA_INVALID) return GHOSTOS_PAGE_FAULT_INVALID_CAPABILITY;
    if (decision == GHOSTOS_PAGE_FAULT_QUOTA_THROTTLED) {
        if (retry_after_us) *retry_after_us = retry;
        return GHOSTOS_PAGE_FAULT_RATE_LIMITED;
    }
    if (decision == GHOSTOS_PAGE_FAULT_QUOTA_REJECTED) {
        if (retry_after_us) *retry_after_us = UINT64_MAX;
        return GHOSTOS_PAGE_FAULT_RATE_LIMITED;
    }
    bool was_handled = ghostos_page_fault_dispatch(fault);
    if (!was_handled && refund) refund(quota_context, caller, authority);
    if (handled) *handled = was_handled;
    return GHOSTOS_PAGE_FAULT_DISPATCH_OK;
}

ghostos_stack_fault_result ghostos_page_fault_resolve_stack(
    ghostos_address_space *address_space, ghostos_page_fault fault,
    ghostos_page_fault_map_stack_fn map_page, void *context, ghostos_stack_growth *growth) {
    if (!address_space || !map_page || !fault.user || fault.present || fault.reserved_bit)
        return GHOSTOS_STACK_FAULT_INVALID;
    uint64_t page = ghostos_page_fault_page_address(fault), candidate = 0;
    if (ghostos_as_stack_growth_page(address_space, page, &candidate) != GHOSTOS_STACK_OK)
        return GHOSTOS_STACK_FAULT_GROWTH;
    if (!map_page(context, address_space->id, candidate)) return GHOSTOS_STACK_FAULT_MAPPING_FAILED;
    return ghostos_as_grow_stack(address_space, page, growth) == GHOSTOS_STACK_OK ?
        GHOSTOS_STACK_FAULT_OK : GHOSTOS_STACK_FAULT_GROWTH;
}

ghostos_stack_fault_result ghostos_page_fault_resolve_stack_with_ops(
    uint32_t address_space, ghostos_page_fault fault,
    ghostos_page_fault_stack_inspect_fn inspect,
    ghostos_page_fault_map_stack_fn map_page,
    ghostos_page_fault_stack_commit_fn commit,
    void *context, ghostos_stack_growth *growth) {
    if (!address_space || !inspect || !map_page || !commit || !fault.user || fault.present || fault.reserved_bit)
        return GHOSTOS_STACK_FAULT_INVALID;
    uint64_t page = ghostos_page_fault_page_address(fault), candidate = 0;
    ghostos_stack_fault_result result = inspect(context, address_space, page, &candidate);
    if (result != GHOSTOS_STACK_FAULT_OK) return result;
    if (!map_page(context, address_space, candidate)) return GHOSTOS_STACK_FAULT_MAPPING_FAILED;
    return commit(context, address_space, page, growth);
}

ghostos_cow_fault_result_kind ghostos_page_fault_resolve_cow(
    ghostos_address_space *address_space, ghostos_cow_manager *cow,
    ghostos_early_frame_allocator *allocator, ghostos_cow_copy_page_fn copy_page,
    void *context, uint32_t owner, ghostos_page_fault fault,
    ghostos_cow_fault_result *result) {
    if (!result) return GHOSTOS_COW_FAULT_INVALID;
    *result = (ghostos_cow_fault_result){0};
    if (!address_space || !cow || !allocator || !copy_page || !fault.user || !fault.present ||
        fault.reserved_bit || fault.access != GHOSTOS_ACCESS_WRITE) return GHOSTOS_COW_FAULT_INVALID;
    uint64_t page = ghostos_page_fault_page_address(fault), mapped_page = 0, frame = 0;
    if (ghostos_as_cow_mapping(address_space, page, &mapped_page, &frame) != GHOSTOS_AS_OK ||
        ghostos_as_can_replace_cow_page(address_space, page) != GHOSTOS_AS_OK)
        return GHOSTOS_COW_FAULT_ADDRESS_SPACE;
    (void)mapped_page;
    ghostos_cow_write_result write_result;
    if (ghostos_cow_write_fault(cow, owner, frame, allocator, copy_page, context, &write_result) != GHOSTOS_COW_OK)
        return GHOSTOS_COW_FAULT_COW;
    uint64_t new_frame = write_result.kind == GHOSTOS_COW_WRITE_EXCLUSIVE ?
        write_result.old_frame : write_result.new_frame;
    if (ghostos_as_replace_cow_page(address_space, page, new_frame, NULL) != GHOSTOS_AS_OK)
        return GHOSTOS_COW_FAULT_ADDRESS_SPACE;
    *result = (ghostos_cow_fault_result){
        .copied = write_result.kind == GHOSTOS_COW_WRITE_COPIED,
        .old_frame = write_result.old_frame,
        .new_frame = new_frame};
    if (write_result.kind == GHOSTOS_COW_WRITE_EXCLUSIVE) result->old_frame = 0;
    return GHOSTOS_COW_FAULT_OK;
}

ghostos_cow_fault_result_kind ghostos_page_fault_resolve_cow_with_ops(
    uint32_t address_space, ghostos_page_fault fault,
    ghostos_page_fault_cow_lookup_fn lookup,
    ghostos_page_fault_cow_write_fn write,
    ghostos_page_fault_cow_replace_fn replace,
    void *context, ghostos_cow_fault_result *result) {
    if (!result) return GHOSTOS_COW_FAULT_INVALID;
    *result = (ghostos_cow_fault_result){0};
    if (!address_space || !lookup || !write || !replace || !fault.user || !fault.present ||
        fault.reserved_bit || fault.access != GHOSTOS_ACCESS_WRITE) return GHOSTOS_COW_FAULT_INVALID;
    uint64_t page = ghostos_page_fault_page_address(fault), frame = 0;
    ghostos_cow_fault_result_kind status = lookup(context, address_space, page, &frame);
    if (status != GHOSTOS_COW_FAULT_OK) return status;
    status = write(context, address_space, frame, result);
    if (status != GHOSTOS_COW_FAULT_OK) return status;
    uint64_t new_frame = result->copied ? result->new_frame : frame;
    status = replace(context, address_space, page, new_frame);
    if (status != GHOSTOS_COW_FAULT_OK) return status;
    result->new_frame = new_frame;
    if (!result->copied) result->old_frame = 0;
    return GHOSTOS_COW_FAULT_OK;
}
