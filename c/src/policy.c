#include "ghostos/policy.h"

_Static_assert(sizeof(ghostos_policy_id) == 32, "policy ID size");
_Static_assert(sizeof(ghostos_policy_principal_slot) == 34, "ghostos_policy_principal_slot size");
_Static_assert(offsetof(ghostos_policy_principal_slot, id) == 0, "ghostos_policy_principal_slot.id offset");
_Static_assert(offsetof(ghostos_policy_principal_slot, active) == 32, "ghostos_policy_principal_slot.active offset");
_Static_assert(offsetof(ghostos_policy_principal_slot, present) == 33, "ghostos_policy_principal_slot.present offset");
_Static_assert(sizeof(ghostos_policy_object_slot) == 112, "ghostos_policy_object_slot size");
_Static_assert(offsetof(ghostos_policy_object_slot, id) == 0, "ghostos_policy_object_slot.id offset");
_Static_assert(offsetof(ghostos_policy_object_slot, owner) == 32, "ghostos_policy_object_slot.owner offset");
_Static_assert(offsetof(ghostos_policy_object_slot, parent) == 64, "ghostos_policy_object_slot.parent offset");
_Static_assert(offsetof(ghostos_policy_object_slot, revision) == 96, "ghostos_policy_object_slot.revision offset");
_Static_assert(offsetof(ghostos_policy_object_slot, kind) == 104, "ghostos_policy_object_slot.kind offset");
_Static_assert(offsetof(ghostos_policy_object_slot, active) == 105, "ghostos_policy_object_slot.active offset");
_Static_assert(offsetof(ghostos_policy_object_slot, has_owner) == 106, "ghostos_policy_object_slot.has_owner offset");
_Static_assert(offsetof(ghostos_policy_object_slot, has_parent) == 107, "ghostos_policy_object_slot.has_parent offset");
_Static_assert(offsetof(ghostos_policy_object_slot, present) == 108, "ghostos_policy_object_slot.present offset");
_Static_assert(sizeof(ghostos_policy_binding_slot) == 80, "ghostos_policy_binding_slot size");
_Static_assert(offsetof(ghostos_policy_binding_slot, principal) == 0, "ghostos_policy_binding_slot.principal offset");
_Static_assert(offsetof(ghostos_policy_binding_slot, object) == 32, "ghostos_policy_binding_slot.object offset");
_Static_assert(offsetof(ghostos_policy_binding_slot, rights) == 64, "ghostos_policy_binding_slot.rights offset");
_Static_assert(offsetof(ghostos_policy_binding_slot, active) == 72, "ghostos_policy_binding_slot.active offset");
_Static_assert(offsetof(ghostos_policy_binding_slot, present) == 73, "ghostos_policy_binding_slot.present offset");
_Static_assert(sizeof(ghostos_policy_snapshot_view) == 56, "ghostos_policy_snapshot_view size");
_Static_assert(offsetof(ghostos_policy_snapshot_view, epoch) == 0, "ghostos_policy_snapshot_view.epoch offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, principals) == 8, "ghostos_policy_snapshot_view.principals offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, principal_capacity) == 16, "ghostos_policy_snapshot_view.principal_capacity offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, objects) == 24, "ghostos_policy_snapshot_view.objects offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, object_capacity) == 32, "ghostos_policy_snapshot_view.object_capacity offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, bindings) == 40, "ghostos_policy_snapshot_view.bindings offset");
_Static_assert(offsetof(ghostos_policy_snapshot_view, binding_capacity) == 48, "ghostos_policy_snapshot_view.binding_capacity offset");
_Static_assert(sizeof(ghostos_policy_change) == 120, "ghostos_policy_change size");
_Static_assert(offsetof(ghostos_policy_change, principal) == 0, "ghostos_policy_change.principal offset");
_Static_assert(offsetof(ghostos_policy_change, object) == 32, "ghostos_policy_change.object offset");
_Static_assert(offsetof(ghostos_policy_change, related) == 64, "ghostos_policy_change.related offset");
_Static_assert(offsetof(ghostos_policy_change, before) == 96, "ghostos_policy_change.before offset");
_Static_assert(offsetof(ghostos_policy_change, after) == 104, "ghostos_policy_change.after offset");
_Static_assert(offsetof(ghostos_policy_change, kind) == 112, "ghostos_policy_change.kind offset");
_Static_assert(offsetof(ghostos_policy_change, before_active) == 113, "ghostos_policy_change.before_active offset");
_Static_assert(offsetof(ghostos_policy_change, after_active) == 114, "ghostos_policy_change.after_active offset");
_Static_assert(sizeof(ghostos_policy_affected_principal) == 33, "ghostos_policy_affected_principal size");
_Static_assert(offsetof(ghostos_policy_affected_principal, id) == 0, "ghostos_policy_affected_principal.id offset");
_Static_assert(offsetof(ghostos_policy_affected_principal, reason) == 32, "ghostos_policy_affected_principal.reason offset");
_Static_assert(sizeof(ghostos_policy_affected_object) == 34, "ghostos_policy_affected_object size");
_Static_assert(offsetof(ghostos_policy_affected_object, id) == 0, "ghostos_policy_affected_object.id offset");
_Static_assert(offsetof(ghostos_policy_affected_object, kind) == 32, "ghostos_policy_affected_object.kind offset");
_Static_assert(offsetof(ghostos_policy_affected_object, reason) == 33, "ghostos_policy_affected_object.reason offset");
_Static_assert(sizeof(ghostos_policy_report) == 8632, "ghostos_policy_report size");
_Static_assert(offsetof(ghostos_policy_report, before_epoch) == 0, "ghostos_policy_report.before_epoch offset");
_Static_assert(offsetof(ghostos_policy_report, after_epoch) == 8, "ghostos_policy_report.after_epoch offset");
_Static_assert(offsetof(ghostos_policy_report, before_fingerprint) == 16, "ghostos_policy_report.before_fingerprint offset");
_Static_assert(offsetof(ghostos_policy_report, after_fingerprint) == 24, "ghostos_policy_report.after_fingerprint offset");
_Static_assert(offsetof(ghostos_policy_report, changed) == 32, "ghostos_policy_report.changed offset");
_Static_assert(offsetof(ghostos_policy_report, principals) == 33, "ghostos_policy_report.principals offset");
_Static_assert(offsetof(ghostos_policy_report, principal_count) == 4264, "ghostos_policy_report.principal_count offset");
_Static_assert(offsetof(ghostos_policy_report, objects) == 4272, "ghostos_policy_report.objects offset");
_Static_assert(offsetof(ghostos_policy_report, object_count) == 8624, "ghostos_policy_report.object_count offset");

static bool id_equal(ghostos_policy_id a, ghostos_policy_id b)
{
    for (size_t byte = 0; byte < 32; ++byte)
        if (a.bytes[byte] != b.bytes[byte]) return false;
    return true;
}

ghostos_policy_id ghostos_policy_id_from_u64(uint64_t value)
{
    ghostos_policy_id id = {{0}};
    for (size_t byte = 0; byte < 8; ++byte)
        id.bytes[31 - byte] = (uint8_t)(value >> (byte * 8));
    return id;
}

static const ghostos_policy_principal_slot *principal_find(
    const ghostos_policy_snapshot_view *snapshot, ghostos_policy_id id)
{
    for (size_t index = 0; index < snapshot->principal_capacity; ++index) {
        const ghostos_policy_principal_slot *entry = &snapshot->principals[index];
        if (entry->present && id_equal(entry->id, id)) return entry;
    }
    return NULL;
}

static const ghostos_policy_object_slot *object_find(
    const ghostos_policy_snapshot_view *snapshot, ghostos_policy_id id)
{
    for (size_t index = 0; index < snapshot->object_capacity; ++index) {
        const ghostos_policy_object_slot *entry = &snapshot->objects[index];
        if (entry->present && id_equal(entry->id, id)) return entry;
    }
    return NULL;
}

uint32_t ghostos_policy_add_principal(ghostos_policy_principal_slot *slots,
    size_t capacity, ghostos_policy_id id)
{
    size_t free_slot = SIZE_MAX;
    for (size_t index = 0; index < capacity; ++index) {
        if (slots[index].present && id_equal(slots[index].id, id)) return GHOSTOS_POLICY_OK;
        if (!slots[index].present && free_slot == SIZE_MAX) free_slot = index;
    }
    if (free_slot == SIZE_MAX) return GHOSTOS_POLICY_CAPACITY;
    slots[free_slot] = (ghostos_policy_principal_slot){id, true, true};
    return GHOSTOS_POLICY_OK;
}

uint32_t ghostos_policy_add_object(ghostos_policy_object_slot *slots,
    size_t capacity, ghostos_policy_object_slot object)
{
    size_t free_slot = SIZE_MAX;
    for (size_t index = 0; index < capacity; ++index) {
        if (slots[index].present && id_equal(slots[index].id, object.id))
            return GHOSTOS_POLICY_INVALID_CHANGE;
        if (!slots[index].present && free_slot == SIZE_MAX) free_slot = index;
    }
    if (free_slot == SIZE_MAX) return GHOSTOS_POLICY_CAPACITY;
    object.present = true;
    slots[free_slot] = object;
    return GHOSTOS_POLICY_OK;
}

uint32_t ghostos_policy_bind(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_binding_slot *slots, size_t capacity, ghostos_policy_binding_slot binding)
{
    if (principal_find(snapshot, binding.principal) == NULL) return GHOSTOS_POLICY_UNKNOWN_PRINCIPAL;
    if (object_find(snapshot, binding.object) == NULL) return GHOSTOS_POLICY_UNKNOWN_OBJECT;
    size_t free_slot = SIZE_MAX;
    binding.present = true;
    for (size_t index = 0; index < capacity; ++index) {
        if (slots[index].present && id_equal(slots[index].principal, binding.principal) &&
            id_equal(slots[index].object, binding.object)) {
            slots[index] = binding;
            return GHOSTOS_POLICY_OK;
        }
        if (!slots[index].present && free_slot == SIZE_MAX) free_slot = index;
    }
    if (free_slot == SIZE_MAX) return GHOSTOS_POLICY_CAPACITY;
    slots[free_slot] = binding;
    return GHOSTOS_POLICY_OK;
}

bool ghostos_policy_find_binding(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_id principal, ghostos_policy_id object, ghostos_policy_binding_slot *out)
{
    for (size_t index = 0; index < snapshot->binding_capacity; ++index) {
        const ghostos_policy_binding_slot *entry = &snapshot->bindings[index];
        if (entry->present && id_equal(entry->principal, principal) && id_equal(entry->object, object)) {
            *out = *entry;
            return true;
        }
    }
    return false;
}

static uint64_t mix(uint64_t hash, uint64_t value)
{
    return (hash ^ value) * UINT64_C(0x100000001b3);
}

static uint64_t mix_id(uint64_t hash, ghostos_policy_id id)
{
    for (size_t byte = 0; byte < 32; ++byte) hash = mix(hash, id.bytes[byte]);
    return hash;
}

uint64_t ghostos_policy_fingerprint(const ghostos_policy_snapshot_view *snapshot)
{
    uint64_t hash = mix(UINT64_C(0xcbf29ce484222325), snapshot->epoch);
    for (size_t index = 0; index < snapshot->principal_capacity; ++index) {
        const ghostos_policy_principal_slot *entry = &snapshot->principals[index];
        if (!entry->present) continue;
        hash = mix(mix_id(hash, entry->id), entry->active);
    }
    for (size_t index = 0; index < snapshot->object_capacity; ++index) {
        const ghostos_policy_object_slot *entry = &snapshot->objects[index];
        if (!entry->present) continue;
        hash = mix(mix_id(hash, entry->id), entry->kind);
        if (entry->has_owner) hash = mix_id(hash, entry->owner);
        if (entry->has_parent) hash = mix_id(hash, entry->parent);
        hash = mix(mix(hash, entry->revision), entry->active);
    }
    for (size_t index = 0; index < snapshot->binding_capacity; ++index) {
        const ghostos_policy_binding_slot *entry = &snapshot->bindings[index];
        if (!entry->present) continue;
        hash = mix_id(mix_id(hash, entry->principal), entry->object);
        hash = mix(mix(hash, entry->rights), entry->active);
    }
    return hash;
}

static uint32_t add_principal(ghostos_policy_report *report, ghostos_policy_id id, uint8_t reason)
{
    for (size_t index = 0; index < report->principal_count; ++index)
        if (id_equal(report->principals[index].id, id)) return GHOSTOS_POLICY_OK;
    if (report->principal_count == GHOSTOS_MAX_POLICY_AFFECTED) return GHOSTOS_POLICY_TOO_MANY_AFFECTED;
    report->principals[report->principal_count++] = (ghostos_policy_affected_principal){id, reason};
    return GHOSTOS_POLICY_OK;
}

static uint32_t add_object(ghostos_policy_report *report, ghostos_policy_id id, uint8_t kind, uint8_t reason)
{
    for (size_t index = 0; index < report->object_count; ++index)
        if (id_equal(report->objects[index].id, id)) return GHOSTOS_POLICY_OK;
    if (report->object_count == GHOSTOS_MAX_POLICY_AFFECTED) return GHOSTOS_POLICY_TOO_MANY_AFFECTED;
    report->objects[report->object_count++] = (ghostos_policy_affected_object){id, kind, reason};
    return GHOSTOS_POLICY_OK;
}

static uint32_t bound_principals(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_id object, uint8_t reason, ghostos_policy_report *report)
{
    for (size_t index = 0; index < snapshot->binding_capacity; ++index) {
        const ghostos_policy_binding_slot *entry = &snapshot->bindings[index];
        if (entry->present && entry->active && id_equal(entry->object, object)) {
            uint32_t code = add_principal(report, entry->principal, reason);
            if (code != GHOSTOS_POLICY_OK) return code;
        }
    }
    return GHOSTOS_POLICY_OK;
}

static uint32_t require_kind(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_id id, uint8_t kind, const ghostos_policy_object_slot **out)
{
    *out = object_find(snapshot, id);
    if (*out == NULL) return GHOSTOS_POLICY_UNKNOWN_OBJECT;
    return (*out)->kind == kind ? GHOSTOS_POLICY_OK : GHOSTOS_POLICY_INVALID_OBJECT_KIND;
}

uint32_t ghostos_policy_simulate(const ghostos_policy_snapshot_view *snapshot,
    const ghostos_policy_change *change, ghostos_policy_report *report)
{
    uint64_t fingerprint = ghostos_policy_fingerprint(snapshot);
    *report = (ghostos_policy_report){.before_epoch = snapshot->epoch, .after_epoch = snapshot->epoch,
        .before_fingerprint = fingerprint, .after_fingerprint = fingerprint};
    const ghostos_policy_object_slot *object, *related;
    ghostos_policy_binding_slot binding;
    uint32_t code;
    switch (change->kind) {
        case GHOSTOS_CHANGE_CAPABILITY: {
            if (principal_find(snapshot, change->principal) == NULL) return GHOSTOS_POLICY_UNKNOWN_PRINCIPAL;
            object = object_find(snapshot, change->object);
            if (object == NULL) return GHOSTOS_POLICY_UNKNOWN_OBJECT;
            uint64_t rights = ghostos_policy_find_binding(snapshot, change->principal, change->object, &binding) ?
                binding.rights : 0;
            if (rights != change->before) return GHOSTOS_POLICY_STALE_SNAPSHOT;
            if (change->before == change->after) return GHOSTOS_POLICY_OK;
            report->changed = true;
            code = add_principal(report, change->principal, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            return add_object(report, object->id, object->kind, change->kind);
        }
        case GHOSTOS_CHANGE_FIREWALL:
            code = require_kind(snapshot, change->object, GHOSTOS_POLICY_FIREWALL_RULE, &object);
            if (code != GHOSTOS_POLICY_OK) return code;
            code = require_kind(snapshot, change->related, GHOSTOS_POLICY_NETWORK, &related);
            if (code != GHOSTOS_POLICY_OK) return code;
            if (object->revision != change->before) return GHOSTOS_POLICY_STALE_SNAPSHOT;
            if (change->before == change->after) return GHOSTOS_POLICY_OK;
            report->changed = true;
            code = add_object(report, object->id, object->kind, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            code = add_object(report, related->id, related->kind, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            return bound_principals(snapshot, related->id, change->kind, report);
        case GHOSTOS_CHANGE_PACKAGE:
            code = require_kind(snapshot, change->object, GHOSTOS_POLICY_PACKAGE, &object);
            if (code != GHOSTOS_POLICY_OK) return code;
            if (change->after == 0) return GHOSTOS_POLICY_INVALID_CHANGE;
            if (object->revision != change->before) return GHOSTOS_POLICY_STALE_SNAPSHOT;
            if (change->before == change->after && object->active == change->after_active)
                return GHOSTOS_POLICY_OK;
            report->changed = true;
            code = add_object(report, object->id, object->kind, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            for (size_t index = 0; index < snapshot->object_capacity; ++index) {
                const ghostos_policy_object_slot *child = &snapshot->objects[index];
                if (child->present && child->has_parent && id_equal(child->parent, object->id)) {
                    code = add_object(report, child->id, child->kind, change->kind);
                    if (code != GHOSTOS_POLICY_OK) return code;
                    code = bound_principals(snapshot, child->id, change->kind, report);
                    if (code != GHOSTOS_POLICY_OK) return code;
                }
            }
            return bound_principals(snapshot, object->id, change->kind, report);
        case GHOSTOS_CHANGE_MEMBERSHIP:
            code = require_kind(snapshot, change->object, GHOSTOS_POLICY_CLUSTER, &object);
            if (code != GHOSTOS_POLICY_OK) return code;
            code = require_kind(snapshot, change->related, GHOSTOS_POLICY_CLUSTER_MEMBER, &related);
            if (code != GHOSTOS_POLICY_OK) return code;
            if (related->active != change->before_active) return GHOSTOS_POLICY_STALE_SNAPSHOT;
            if (change->before_active == change->after_active) return GHOSTOS_POLICY_OK;
            report->changed = true;
            code = add_object(report, object->id, GHOSTOS_POLICY_CLUSTER, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            code = add_object(report, related->id, related->kind, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            return bound_principals(snapshot, related->id, change->kind, report);
        case GHOSTOS_CHANGE_UPDATE:
            code = require_kind(snapshot, change->object, GHOSTOS_POLICY_UPDATE, &object);
            if (code != GHOSTOS_POLICY_OK) return code;
            if (object->revision != change->before) return GHOSTOS_POLICY_STALE_SNAPSHOT;
            if (change->before == change->after) return GHOSTOS_POLICY_OK;
            if (change->after == 0) return GHOSTOS_POLICY_INVALID_CHANGE;
            report->changed = true;
            code = add_object(report, object->id, GHOSTOS_POLICY_UPDATE, change->kind);
            if (code != GHOSTOS_POLICY_OK) return code;
            for (size_t index = 0; index < snapshot->object_capacity; ++index) {
                const ghostos_policy_object_slot *entry = &snapshot->objects[index];
                if (entry->present && entry->active && entry->revision == change->before &&
                    !id_equal(entry->id, object->id)) {
                    code = add_object(report, entry->id, entry->kind, change->kind);
                    if (code != GHOSTOS_POLICY_OK) return code;
                    code = bound_principals(snapshot, entry->id, change->kind, report);
                    if (code != GHOSTOS_POLICY_OK) return code;
                }
            }
            return GHOSTOS_POLICY_OK;
        default: return GHOSTOS_POLICY_INVALID_CHANGE;
    }
}
