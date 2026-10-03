#include "ghostos/policy.h"
#include <assert.h>

/* Ports of the three retained Rust policy fixtures. */
typedef struct {
    ghostos_policy_principal_slot principals[8];
    ghostos_policy_object_slot objects[16];
    ghostos_policy_binding_slot bindings[16];
    ghostos_policy_snapshot_view view;
} fixture;

static void fixture_init(fixture *state, uint64_t epoch)
{
    *state = (fixture){0};
    state->view = (ghostos_policy_snapshot_view){epoch,
        state->principals, 8, state->objects, 16, state->bindings, 16};
}

static void add_object(fixture *state, uint64_t id, uint8_t kind, bool has_owner,
    uint64_t parent, uint64_t revision, bool active)
{
    ghostos_policy_object_slot object = {
        .id = ghostos_policy_id_from_u64(id), .kind = kind,
        .owner = ghostos_policy_id_from_u64(1), .has_owner = has_owner,
        .parent = ghostos_policy_id_from_u64(parent), .has_parent = parent != 0,
        .revision = revision, .active = active
    };
    assert(ghostos_policy_add_object(state->objects, 16, object) == GHOSTOS_POLICY_OK);
}

static void bind(fixture *state, uint64_t object)
{
    ghostos_policy_binding_slot binding = {
        .principal = ghostos_policy_id_from_u64(1),
        .object = ghostos_policy_id_from_u64(object), .rights = 1, .active = true
    };
    assert(ghostos_policy_bind(&state->view, state->bindings, 16, binding) == GHOSTOS_POLICY_OK);
}

static void snapshot(fixture *state)
{
    fixture_init(state, 7);
    assert(ghostos_policy_add_principal(state->principals, 8,
        ghostos_policy_id_from_u64(1)) == GHOSTOS_POLICY_OK);
    add_object(state, 2, GHOSTOS_POLICY_CAPABILITY, true, 0, 3, true);
    bind(state, 2);
}

static bool read_only(const ghostos_policy_report *report)
{
    return report->before_epoch == report->after_epoch &&
        report->before_fingerprint == report->after_fingerprint;
}

static void simulation_does_not_mutate_snapshot(void)
{
    fixture state;
    snapshot(&state);
    uint64_t epoch = state.view.epoch;
    ghostos_policy_change change = {.kind = GHOSTOS_CHANGE_CAPABILITY,
        .principal = ghostos_policy_id_from_u64(1), .object = ghostos_policy_id_from_u64(2),
        .before = 1, .after = 0};
    ghostos_policy_report report;
    assert(ghostos_policy_simulate(&state.view, &change, &report) == GHOSTOS_POLICY_OK);
    assert(report.changed && read_only(&report));
    assert(state.view.epoch == epoch);
    ghostos_policy_binding_slot binding;
    assert(ghostos_policy_find_binding(&state.view, change.principal, change.object, &binding));
    assert(binding.rights == 1);
}

static void package_simulation_reports_children_and_principals(void)
{
    fixture state;
    snapshot(&state);
    add_object(&state, 3, GHOSTOS_POLICY_PACKAGE, false, 0, 4, true);
    add_object(&state, 4, GHOSTOS_POLICY_PACKAGE_PROCESS, true, 3, 4, true);
    bind(&state, 3);
    ghostos_policy_change change = {.kind = GHOSTOS_CHANGE_PACKAGE,
        .object = ghostos_policy_id_from_u64(3), .before = 4, .after = 5, .after_active = false};
    ghostos_policy_report report;
    assert(ghostos_policy_simulate(&state.view, &change, &report) == GHOSTOS_POLICY_OK);
    assert(report.object_count == 2 && report.principal_count == 1);
}

static void simulation_covers_firewall_membership_and_rollout(void)
{
    fixture state;
    fixture_init(&state, 9);
    assert(ghostos_policy_add_principal(state.principals, 8,
        ghostos_policy_id_from_u64(1)) == GHOSTOS_POLICY_OK);
    add_object(&state, 10, GHOSTOS_POLICY_FIREWALL_RULE, true, 0, 1, true);
    add_object(&state, 11, GHOSTOS_POLICY_NETWORK, true, 0, 1, true);
    add_object(&state, 20, GHOSTOS_POLICY_CLUSTER, true, 0, 1, true);
    add_object(&state, 21, GHOSTOS_POLICY_CLUSTER_MEMBER, true, 20, 1, true);
    add_object(&state, 30, GHOSTOS_POLICY_UPDATE, true, 0, 1, true);
    bind(&state, 11);
    bind(&state, 21);
    ghostos_policy_change firewall = {.kind = GHOSTOS_CHANGE_FIREWALL,
        .object = ghostos_policy_id_from_u64(10), .related = ghostos_policy_id_from_u64(11),
        .before = 1, .after = 2};
    ghostos_policy_change membership = {.kind = GHOSTOS_CHANGE_MEMBERSHIP,
        .object = ghostos_policy_id_from_u64(20), .related = ghostos_policy_id_from_u64(21),
        .before_active = true, .after_active = false};
    ghostos_policy_change rollout = {.kind = GHOSTOS_CHANGE_UPDATE,
        .object = ghostos_policy_id_from_u64(30), .before = 1, .after = 2};
    ghostos_policy_report report;
    assert(ghostos_policy_simulate(&state.view, &firewall, &report) == GHOSTOS_POLICY_OK);
    assert(report.changed && read_only(&report));
    assert(ghostos_policy_simulate(&state.view, &membership, &report) == GHOSTOS_POLICY_OK);
    assert(report.changed && read_only(&report) && report.principal_count == 1);
    assert(ghostos_policy_simulate(&state.view, &rollout, &report) == GHOSTOS_POLICY_OK);
    assert(report.changed && read_only(&report) && report.object_count >= 4);
}

int main(void)
{
    simulation_does_not_mutate_snapshot();
    package_simulation_reports_children_and_principals();
    simulation_covers_firewall_membership_and_rollout();
    return 0;
}
