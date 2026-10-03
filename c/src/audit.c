#include "ghostos/audit.h"
_Static_assert(sizeof(ghostos_audit_advisory) == 99, "audit advisory ABI");
_Static_assert(offsetof(ghostos_audit_advisory, occupied) == 98, "audit advisory occupancy ABI");
_Static_assert(sizeof(ghostos_audit_obsolete_entry) == 40, "audit obsolete ABI");
_Static_assert(offsetof(ghostos_audit_obsolete_entry, occupied) == 37, "audit obsolete occupancy ABI");
_Static_assert(sizeof(ghostos_audit_finding) == 133, "audit finding ABI");
_Static_assert(offsetof(ghostos_audit_finding, occupied) == 132, "audit finding occupancy ABI");
static bool same_bytes(const uint8_t *left, const uint8_t *right, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) if (left[i] != right[i]) return false;
    return true;
}
int ghostos_audit_id(const uint8_t *id, size_t length) {
    size_t i;
    if (!length || length > 64) return 1;
    for (i = 0; i < length; ++i) if (id[i] > 127) return 1;
    return 0;
}
int ghostos_audit_advisory_slot(const ghostos_audit_advisory *records, size_t count,
    uint8_t source, const uint8_t *id, size_t id_length, const uint8_t package_hash[32],
    size_t *index) {
    size_t free_slot = count;
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (records[i].source == source && records[i].id_length == id_length &&
            same_bytes(records[i].id, id, id_length) &&
            same_bytes(records[i].package_hash, package_hash, 32)) return 1;
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 0;
}
int ghostos_audit_obsolete(bool name_empty, uint8_t reason, uint32_t installed_major,
    uint32_t installed_minor, uint32_t installed_patch, uint32_t latest_major,
    uint32_t latest_minor, uint32_t latest_patch) {
    int order = 0;
    if (name_empty) return 1;
    if (reason != 2) return 0;
    if (installed_major != latest_major) order = installed_major < latest_major ? -1 : 1;
    else if (installed_minor != latest_minor) order = installed_minor < latest_minor ? -1 : 1;
    else if (installed_patch != latest_patch) order = installed_patch < latest_patch ? -1 : 1;
    return order < 0 ? 0 : 1;
}
int ghostos_audit_obsolete_slot(const ghostos_audit_obsolete_entry *records, size_t count,
    uint32_t node, const uint8_t package[32], uint8_t reason, size_t *index) {
    size_t free_slot = count;
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (records[i].node == node && records[i].reason == reason &&
            same_bytes(records[i].package, package, 32)) return 1;
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 0;
}
int ghostos_audit_finding_slot(const ghostos_audit_finding *records, size_t count,
    const uint8_t package[32], uint8_t source, const uint8_t *id, size_t id_length,
    const uint8_t advisory_package[32], uint8_t severity, bool withdrawn, size_t *index) {
    size_t free_slot = count;
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (same_bytes(records[i].package, package, 32) && records[i].source == source &&
            records[i].id_length == id_length && same_bytes(records[i].id, id, id_length) &&
            same_bytes(records[i].advisory_package, advisory_package, 32) &&
            records[i].severity == severity && records[i].withdrawn == withdrawn) return 1;
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 0;
}
int ghostos_audit_budget(size_t package_budget) {
    return package_budget ? 0 : 1;
}
