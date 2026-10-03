#ifndef GHOSTOS_POLICY_H
#define GHOSTOS_POLICY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_POLICY_PRINCIPALS 64
#define GHOSTOS_MAX_POLICY_OBJECTS 128
#define GHOSTOS_MAX_POLICY_BINDINGS 256
#define GHOSTOS_MAX_POLICY_AFFECTED 128

typedef struct { uint8_t bytes[32]; } ghostos_policy_id;
enum ghostos_policy_object_kind {
    GHOSTOS_POLICY_CAPABILITY = 1, GHOSTOS_POLICY_FIREWALL_RULE,
    GHOSTOS_POLICY_NETWORK, GHOSTOS_POLICY_PACKAGE, GHOSTOS_POLICY_PACKAGE_PROCESS,
    GHOSTOS_POLICY_CLUSTER, GHOSTOS_POLICY_CLUSTER_MEMBER, GHOSTOS_POLICY_UPDATE,
    GHOSTOS_POLICY_SYSTEM
};
enum ghostos_policy_change_kind {
    GHOSTOS_CHANGE_CAPABILITY, GHOSTOS_CHANGE_FIREWALL, GHOSTOS_CHANGE_PACKAGE,
    GHOSTOS_CHANGE_MEMBERSHIP, GHOSTOS_CHANGE_UPDATE
};
enum ghostos_policy_error {
    GHOSTOS_POLICY_OK, GHOSTOS_POLICY_CAPACITY, GHOSTOS_POLICY_UNKNOWN_PRINCIPAL,
    GHOSTOS_POLICY_UNKNOWN_OBJECT, GHOSTOS_POLICY_INVALID_OBJECT_KIND,
    GHOSTOS_POLICY_INVALID_CHANGE, GHOSTOS_POLICY_STALE_SNAPSHOT,
    GHOSTOS_POLICY_TOO_MANY_AFFECTED
};
/* Caller-owned slot arrays are initially zeroed. Presence is independent of
 * active state. IDs may contain all zero bytes, matching Rust constructors. */
typedef struct {
    ghostos_policy_id id;
    bool active, present;
} ghostos_policy_principal_slot;
typedef struct {
    ghostos_policy_id id, owner, parent;
    uint64_t revision;
    uint8_t kind;
    bool active, has_owner, has_parent, present;
} ghostos_policy_object_slot;
typedef struct {
    ghostos_policy_id principal, object;
    uint64_t rights;
    bool active, present;
} ghostos_policy_binding_slot;
typedef struct {
    uint64_t epoch;
    const ghostos_policy_principal_slot *principals;
    size_t principal_capacity;
    const ghostos_policy_object_slot *objects;
    size_t object_capacity;
    const ghostos_policy_binding_slot *bindings;
    size_t binding_capacity;
} ghostos_policy_snapshot_view;
/* Unused fields are ignored. object/related mean rule/network for firewall,
 * cluster/member for membership, and the changed object for other changes.
 * before/after encode rights or revision. Package uses after_active only. */
typedef struct {
    ghostos_policy_id principal, object, related;
    uint64_t before, after;
    uint8_t kind;
    bool before_active, after_active;
} ghostos_policy_change;
typedef struct { ghostos_policy_id id; uint8_t reason; } ghostos_policy_affected_principal;
typedef struct { ghostos_policy_id id; uint8_t kind, reason; } ghostos_policy_affected_object;
typedef struct {
    uint64_t before_epoch, after_epoch, before_fingerprint, after_fingerprint;
    bool changed;
    ghostos_policy_affected_principal principals[GHOSTOS_MAX_POLICY_AFFECTED];
    size_t principal_count;
    ghostos_policy_affected_object objects[GHOSTOS_MAX_POLICY_AFFECTED];
    size_t object_count;
} ghostos_policy_report;

ghostos_policy_id ghostos_policy_id_from_u64(uint64_t value);
uint32_t ghostos_policy_add_principal(ghostos_policy_principal_slot *slots,
    size_t capacity, ghostos_policy_id id);
/* Duplicate object rejection precedes capacity; owner/parent need not exist. */
uint32_t ghostos_policy_add_object(ghostos_policy_object_slot *slots,
    size_t capacity, ghostos_policy_object_slot object);
/* Principal lookup precedes object lookup; replacement does not require space. */
uint32_t ghostos_policy_bind(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_binding_slot *slots, size_t capacity, ghostos_policy_binding_slot binding);
bool ghostos_policy_find_binding(const ghostos_policy_snapshot_view *snapshot,
    ghostos_policy_id principal, ghostos_policy_id object, ghostos_policy_binding_slot *out);
uint64_t ghostos_policy_fingerprint(const ghostos_policy_snapshot_view *snapshot);
/* Simulation is read-only. Only successful reports may be consumed. Descendants
 * are direct children, including inactive children. De-duplication keeps the
 * first occurrence and its reason. No pointers are retained by any operation. */
uint32_t ghostos_policy_simulate(const ghostos_policy_snapshot_view *snapshot,
    const ghostos_policy_change *change, ghostos_policy_report *report);

#endif
