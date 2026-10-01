#include "ghostos/address_space.h"
#include "ghostos/boot_protocol.h"
#include "ghostos/capability.h"
#include "ghostos/invariants.h"
#include "ghostos/kernel.h"
#include "ghostos/runtime.h"
#include "ghostos/scheduler.h"
#include "ghostos/status.h"
#include "ghostos/syscall.h"
#include "ghostos/task.h"
#include "ghostos/tlb.h"
#include "ghostos/test_property.h"

#include <assert.h>
#include <stdint.h>

static void boot_info_rejects_too_many_regions(void) {
    ghostos_boot_info info = ghostos_boot_info_empty(GHOSTOS_BOOT_UEFI);
    info.memory_region_count = GHOSTOS_MAX_MEMORY_REGIONS + 1;
    assert(ghostos_kernel_validate_boot_info(&info) == GHOSTOS_STATUS_INVALID_ARGUMENT);
}

static void address_spaces_keep_private_mappings(void) {
    ghostos_page_table_root first_root, second_root;
    ghostos_address_space_table table;
    ghostos_mapping first_mapping, second_mapping;
    assert(ghostos_as_page_table_root(0x1000, &first_root));
    assert(ghostos_as_page_table_root(0x2000, &second_root));
    ghostos_as_table_init(&table);
    assert(ghostos_as_table_create(&table, 41, first_root, 1) == GHOSTOS_AS_OK);
    assert(ghostos_as_table_create(&table, 42, second_root, 2) == GHOSTOS_AS_OK);
    ghostos_address_space *first = ghostos_as_table_get(&table, 41);
    ghostos_address_space *second = ghostos_as_table_get(&table, 42);
    assert(first && second);
    assert(ghostos_as_map_backing(first, 1, (ghostos_physical_range){0x400000, 0x1000}, true,
        &first_mapping) == GHOSTOS_AS_OK);
    assert(ghostos_as_map_backing(second, 2, (ghostos_physical_range){0x500000, 0x1000}, true,
        &second_mapping) == GHOSTOS_AS_OK);
    assert(ghostos_as_table_check_isolation(&table, 41, 42) == GHOSTOS_ISOLATION_OK);
    assert(ghostos_as_can_access(first, first_mapping.base, 0x1000, GHOSTOS_ACCESS_READ));
    assert(ghostos_as_can_access(first, first_mapping.base, 0x1000, GHOSTOS_ACCESS_WRITE));
    assert(ghostos_as_can_access(second, second_mapping.base, 0x1000, GHOSTOS_ACCESS_READ));
    assert(ghostos_as_can_access(second, second_mapping.base, 0x1000, GHOSTOS_ACCESS_WRITE));
}

static void capability_delegation_attenuates_and_revokes_descendants(void) {
    ghostos_capability_space caps;
    uint64_t root, child, grandchild;
    size_t revoked = 0;
    ghostos_capability_info inspected;
    ghostos_capability_object object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_ADDRESS_SPACE, 0, 1, 0, 0);
    ghostos_capability_space_init(&caps, 4, NULL, NULL);
    assert(ghostos_capability_mint_root(&caps, 1, object,
        GHOSTOS_RIGHT_READ | GHOSTOS_RIGHT_WRITE | GHOSTOS_RIGHT_DELEGATE |
            GHOSTOS_RIGHT_REVOKE, &root) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_delegate(&caps, 1, root, 2, GHOSTOS_RIGHT_READ,
        &child) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_authorize(&caps, 2, child, object,
        GHOSTOS_RIGHT_READ) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_delegate(&caps, 1, root, 3, GHOSTOS_RIGHT_WRITE,
        &grandchild) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_revoke(&caps, 1, root, &revoked) == GHOSTOS_CAP_OK);
    assert(revoked == 2);
    assert(ghostos_capability_inspect(&caps, 2, child, &inspected) == GHOSTOS_CAP_INVALID_HANDLE);
    assert(ghostos_capability_inspect(&caps, 3, grandchild, &inspected) == GHOSTOS_CAP_INVALID_HANDLE);
}

static void capability_reuse_changes_generation(void) {
    ghostos_capability_space caps;
    uint64_t first, second;
    size_t revoked = 0;
    ghostos_capability_info inspected;
    ghostos_capability_object object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_ADDRESS_SPACE, 0, 1, 0, 0);
    ghostos_capability_space_init(&caps, 1, NULL, NULL);
    assert(ghostos_capability_mint_root(&caps, 1, object, GHOSTOS_RIGHT_READ,
        &first) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_delete(&caps, 1, first, &revoked) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_mint_root(&caps, 1, object, GHOSTOS_RIGHT_READ,
        &second) == GHOSTOS_CAP_OK);
    assert(first != second);
    assert(ghostos_capability_inspect(&caps, 1, first, &inspected) == GHOSTOS_CAP_INVALID_HANDLE);
}

static void scheduler_picks_earliest_realtime_deadline(void) {
    const ghostos_scheduler_thread_view threads[] = {
        {1, GHOSTOS_SCHEDULER_THREAD_READY, GHOSTOS_SCHEDULER_POLICY_REALTIME,
            20, 0, 100, UINT64_MAX, {1, 0}},
        {2, GHOSTOS_SCHEDULER_THREAD_READY, GHOSTOS_SCHEDULER_POLICY_REALTIME,
            20, 0, 50, UINT64_MAX, {1, 0}},
    };
    const uint64_t cpus[2] = {1, 0};
    assert(ghostos_scheduler_pick_realtime(threads, 2, cpus) == 1);
}

static void invariants_are_stable_and_redacted(void) {
    ghostos_invariant_failure failure;
    const char *formatted = ghostos_invariant_identifier(
        GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION);
    uint64_t frames[] = {0x1000, 0x1000, 0x3000, 0x4000, 0x5000, 0x6000};
    assert(GHOSTOS_INVARIANT_CATALOGUE_COUNT == 6);
    assert(formatted && formatted[0] != '\0');
    assert(!ghostos_invariant_check_page_table_transition(frames, 6, 0, &failure));
    assert(failure.invariant == GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION);
}

static void syscall_boundary_checks_match_kernel_ranges(void) {
    uint64_t args[6] = {0};
    assert(ghostos_syscall_valid_user_range(GHOSTOS_USER_SPACE_START, 64, 8));
    assert(!ghostos_syscall_valid_user_range(GHOSTOS_USER_SPACE_START - 1, 64, 8));
    assert(ghostos_syscall_validate_request_shape(GHOSTOS_OP_YIELD,
        GHOSTOS_ABI_SCHEMA_VERSION, 0));
    assert(!ghostos_syscall_validate_request_shape(UINT16_MAX,
        GHOSTOS_ABI_SCHEMA_VERSION, 0));
    args[2] = 1;
    assert(ghostos_syscall_validate_memory_map(0, 0, args));
    args[3] = 1;
    assert(!ghostos_syscall_validate_memory_map(0, 0, args));
    assert(ghostos_syscall_sleep_hint(GHOSTOS_OP_SLEEP_UNTIL,
        GHOSTOS_STATUS_NORMAL, 50, 50) == UINT64_MAX - 1);
}

static void runtime_rejects_unknown_and_reserved_requests(void) {
    ghostos_request request = {0};
    request.abi_version = GHOSTOS_ABI_SCHEMA_VERSION;
    request.operation = UINT16_MAX;
    assert(ghostos_runtime_validate_request(&request) == GHOSTOS_RUNTIME_INVALID_REQUEST);
    request.operation = GHOSTOS_OP_CLOCK_NOW;
    request.reserved = 1;
    assert(ghostos_runtime_validate_request(&request) == GHOSTOS_RUNTIME_INVALID_REQUEST);
}

typedef struct { size_t invalidations, ipis; } tlb_test_counters;

static void count_tlb_invalidation(void *context, uint64_t start, uint64_t length) {
    tlb_test_counters *counters = context;
    assert(start == 0x4000 && length == 0x2000);
    ++counters->invalidations;
}

static void count_tlb_ipi(void *context, uint8_t cpu) {
    tlb_test_counters *counters = context;
    assert(cpu == 1);
    ++counters->ipis;
}

static void tlb_tracks_targets_until_acknowledged(void) {
    ghostos_tlb_state state;
    const uint64_t targets[] = {3, 0};
    const uint64_t missing_initiator_targets[] = {2, 0};
    tlb_test_counters counters = {0};
    uint64_t id = 0, pending[2] = {0};
    bool complete = false;
    ghostos_tlb_init(&state, 2);
    assert(ghostos_tlb_begin(&state, 9, 0x4000, 0x2000, missing_initiator_targets,
        0, count_tlb_invalidation, count_tlb_ipi, &counters, &id) == GHOSTOS_TLB_MISSING_INITIATOR);
    assert(ghostos_tlb_begin(&state, 9, 0x4000, 0x2000, targets, 0,
        count_tlb_invalidation, count_tlb_ipi, &counters, &id) == GHOSTOS_TLB_OK);
    assert(id == 1 && counters.invalidations == 1 && counters.ipis == 1);
    assert(ghostos_tlb_pending_targets(&state, id, pending));
    assert(pending[0] == 2 && pending[1] == 0);
    assert(ghostos_tlb_acknowledge(&state, id, 0, count_tlb_invalidation,
        &counters, &complete) == GHOSTOS_TLB_ALREADY_ACKNOWLEDGED);
    assert(ghostos_tlb_acknowledge(&state, id, 1, count_tlb_invalidation,
        &counters, &complete) == GHOSTOS_TLB_OK);
    assert(complete && ghostos_tlb_is_complete(&state, id));
    assert(counters.invalidations == 2);
    assert(ghostos_tlb_retire(&state, id) == GHOSTOS_TLB_OK);
    assert(ghostos_tlb_retire(&state, id) == GHOSTOS_TLB_NOT_FOUND);
}

static bool delegated_rights_stay_attenuated(size_t case_index, uint64_t case_seed,
    ghostos_test_entropy *entropy, void *context) {
    (void)case_index;
    (void)case_seed;
    (void)context;
    ghostos_capability_space caps;
    ghostos_capability_handle root, delegated;
    uint16_t requested = (uint16_t)(ghostos_test_entropy_next_u64(entropy) &
        GHOSTOS_RIGHT_ALL) | GHOSTOS_RIGHT_READ;
    ghostos_capability_object object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_ADDRESS_SPACE, 0, 1, 0, 0);
    ghostos_capability_space_init(&caps, 2, NULL, NULL);
    if (ghostos_capability_mint_root(&caps, 1, object, GHOSTOS_RIGHT_ALL,
            &root) != GHOSTOS_CAP_OK ||
        ghostos_capability_delegate(&caps, 1, root, 2, requested,
            &delegated) != GHOSTOS_CAP_OK ||
        ghostos_capability_authorize(&caps, 2, delegated, object,
            requested) != GHOSTOS_CAP_OK)
        return false;
    uint16_t extra = (uint16_t)(GHOSTOS_RIGHT_ALL & ~requested);
    return extra == 0 || ghostos_capability_authorize(&caps, 2, delegated,
        object, extra) != GHOSTOS_CAP_OK;
}

int main(void) {
    boot_info_rejects_too_many_regions();
    address_spaces_keep_private_mappings();
    capability_delegation_attenuates_and_revokes_descendants();
    capability_reuse_changes_generation();
    scheduler_picks_earliest_realtime_deadline();
    invariants_are_stable_and_redacted();
    syscall_boundary_checks_match_kernel_ranges();
    runtime_rejects_unknown_and_reserved_requests();
    tlb_tracks_targets_until_acknowledged();
    ghostos_property_failure failure;
    ghostos_property_config property = ghostos_property_config_new(0x593, 128);
    assert(ghostos_property_run_assert("kernel.capability-attenuation", property,
        delegated_rights_stay_attenuated, NULL, &failure) == GHOSTOS_PROPERTY_OK);
    ghostos_property_failure_dispose(&failure);
    assert(ghostos_kernel_service_image_fits(GHOSTOS_KERNEL_SERVICE_CODE_BYTES));
    assert(!ghostos_kernel_service_image_fits(GHOSTOS_KERNEL_SERVICE_CODE_BYTES + 1));
    return 0;
}
