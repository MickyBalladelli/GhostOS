#include "ghostos/admission.h"
#include <assert.h>

enum { FANOUT, MEMBERSHIP, SNAPSHOT, BACKUP, PACKAGES, DIAGNOSTICS };
enum { OPTIONAL, NORMAL, CRITICAL, RECOVERY };
enum { ADMITTED = 1, DELAYED, DROPPED, RETRIED };

static ghostos_admission_outcome admit(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, uint64_t tenant,
    uint8_t class_id, uint8_t priority) {
    ghostos_admission_outcome result;
    assert(ghostos_admission_admit(state, slots, capacity, tenant, class_id, priority, &result) == 0);
    return result;
}

static void recovery_uses_reserved_capacity_when_optional_work_is_blocked(void) {
    ghostos_admission_policy policy = {4, 1, 1, {4, 4, 4, 4, 4, 4}};
    ghostos_admission_controller state;
    ghostos_admission_slot slots[4];
    assert(ghostos_admission_init(&state, slots, 4, &policy) == 0);
    for (size_t i = 0; i < 3; ++i) {
        assert(admit(&state, slots, 4, 0, BACKUP, NORMAL).action == ADMITTED);
    }
    assert(admit(&state, slots, 4, 0, DIAGNOSTICS, OPTIONAL).action == DELAYED);
    assert(admit(&state, slots, 4, 0, SNAPSHOT, RECOVERY).action == ADMITTED);
    ghostos_admission_report report;
    ghostos_admission_get_report(&state, &report);
    assert(report.recovery_active == 1);
}

static void full_queue_reports_drop_and_retry_separately(void) {
    ghostos_admission_policy policy = {1, 1, 0, {1, 1, 1, 1, 1, 1}};
    ghostos_admission_controller state;
    ghostos_admission_slot slots[1];
    assert(ghostos_admission_init(&state, slots, 1, &policy) == 0);
    ghostos_admission_outcome first = admit(&state, slots, 1, 0, FANOUT, CRITICAL);
    assert(first.action == ADMITTED && first.has_lease);
    assert(admit(&state, slots, 1, 0, DIAGNOSTICS, OPTIONAL).action == DELAYED);
    assert(admit(&state, slots, 1, 0, DIAGNOSTICS, OPTIONAL).action == DROPPED);
    assert(admit(&state, slots, 1, 0, MEMBERSHIP, CRITICAL).action == RETRIED);
    ghostos_admission_report report;
    ghostos_admission_get_report(&state, &report);
    assert(report.classes[DIAGNOSTICS].dropped == 1);
    assert(report.classes[MEMBERSHIP].retried == 1);
    assert(ghostos_admission_finish(&state, slots, 1, &first.lease) == 0);
}

static void tenant_children_share_parent_recovery_reserve(void) {
    ghostos_admission_policy policy = {4, 2, 0, {4, 4, 4, 4, 4, 4}};
    ghostos_admission_controller state;
    ghostos_admission_slot slots[4];
    assert(ghostos_admission_init(&state, slots, 4, &policy) == 0);
    ghostos_admission_tenant_policy parent = {100, 0, 2, 1, false};
    ghostos_admission_tenant_policy first = {1, 100, 2, 0, true};
    ghostos_admission_tenant_policy second = {2, 100, 2, 0, true};
    assert(ghostos_admission_configure_tenant(&state, 4, &parent) == 0);
    assert(ghostos_admission_configure_tenant(&state, 4, &first) == 0);
    assert(ghostos_admission_configure_tenant(&state, 4, &second) == 0);
    assert(admit(&state, slots, 4, 1, BACKUP, NORMAL).action == ADMITTED);
    assert(admit(&state, slots, 4, 2, BACKUP, NORMAL).reason == 5);
    assert(admit(&state, slots, 4, 2, SNAPSHOT, RECOVERY).action == ADMITTED);
}

int main(void) {
    recovery_uses_reserved_capacity_when_optional_work_is_blocked();
    full_queue_reports_drop_and_retry_separately();
    tenant_children_share_parent_recovery_reserve();
    return 0;
}
