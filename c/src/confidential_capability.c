#include "ghostos/confidential_capability.h"
#include "ghostos/vm_snapshot_auth.h"
static void store_be64(uint8_t *output, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) output[i] = (uint8_t)(value >> (56 - 8 * i));
}
static bool zero_bytes(const uint8_t *bytes, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) if (bytes[i]) return false;
    return true;
}
static bool valid_resource(uint8_t kind, uint64_t resource_a, uint64_t resource_b) {
    if (kind == 1) return !(resource_a % 4096) && !(resource_b % 4096) && resource_b &&
        resource_b <= UINT64_MAX - resource_a;
    return (kind == 2 || kind == 3) && resource_a && resource_a <= UINT32_MAX;
}
static bool contains_rights(uint8_t have, uint8_t required) {
    return (have & required) == required;
}
static void digest(const ghostos_confidential_capability *capability, uint8_t out[32]) {
    uint8_t material[128] = {0};
    size_t i;
    store_be64(material, capability->issuer);
    store_be64(material + 8, capability->epoch);
    store_be64(material + 16, capability->id);
    material[24] = (uint8_t)(capability->node >> 24);
    material[25] = (uint8_t)(capability->node >> 16);
    material[26] = (uint8_t)(capability->node >> 8);
    material[27] = (uint8_t)capability->node;
    store_be64(material + 28, capability->subject);
    material[36] = capability->rights;
    store_be64(material + 37, capability->expires_at_us);
    for (i = 0; i < 32; ++i) material[45 + i] = capability->attestation[i];
    material[77] = capability->kind;
    if (capability->kind == 1) {
        store_be64(material + 78, capability->resource_a);
        store_be64(material + 86, capability->resource_b);
    } else {
        material[78] = (uint8_t)(capability->resource_a >> 24);
        material[79] = (uint8_t)(capability->resource_a >> 16);
        material[80] = (uint8_t)(capability->resource_a >> 8);
        material[81] = (uint8_t)capability->resource_a;
    }
    ghostos_vm_snapshot_sha256(material, sizeof material, out);
}
static bool same_capability(const ghostos_confidential_capability *left, const ghostos_confidential_capability *right) {
    size_t i;
    if (left->issuer != right->issuer || left->epoch != right->epoch || left->id != right->id ||
        left->subject != right->subject || left->expires_at_us != right->expires_at_us ||
        left->resource_a != right->resource_a || left->resource_b != right->resource_b ||
        left->node != right->node || left->kind != right->kind || left->rights != right->rights) return false;
    for (i = 0; i < 32; ++i) if (left->attestation[i] != right->attestation[i] || left->token[i] != right->token[i])
        return false;
    return true;
}
int ghostos_confidential_authority_init(ghostos_confidential_authority *authority, uint64_t issuer) {
    if (!issuer) return 1;
    authority->issuer = issuer;
    authority->epoch = 1;
    authority->next_id = 1;
    return 0;
}
int ghostos_confidential_issue(ghostos_confidential_authority *authority,
    ghostos_confidential_capability *records, size_t count, uint32_t node, uint64_t subject,
    uint8_t kind, uint64_t resource_a, uint64_t resource_b, uint8_t rights, uint64_t expires_at_us,
    uint64_t now_us, const uint8_t attestation[32], size_t *index) {
    size_t free_slot = count;
    size_t i;
    uint64_t next;
    if (!subject || !rights || !expires_at_us || expires_at_us <= now_us || zero_bytes(attestation, 32) ||
        !valid_resource(kind, resource_a, resource_b)) return 1;
    for (i = 0; i < count; ++i) if (!records[i].occupied && free_slot == count) free_slot = i;
    if (free_slot == count) return 2;
    records[free_slot] = (ghostos_confidential_capability){0};
    records[free_slot].issuer = authority->issuer;
    records[free_slot].epoch = authority->epoch;
    records[free_slot].id = authority->next_id;
    records[free_slot].subject = subject;
    records[free_slot].expires_at_us = expires_at_us;
    records[free_slot].resource_a = resource_a;
    records[free_slot].resource_b = resource_b;
    records[free_slot].node = node;
    records[free_slot].kind = kind;
    records[free_slot].rights = rights;
    for (i = 0; i < 32; ++i) records[free_slot].attestation[i] = attestation[i];
    digest(&records[free_slot], records[free_slot].token);
    records[free_slot].occupied = true;
    next = authority->next_id + 1;
    authority->next_id = next ? next : 1;
    *index = free_slot;
    return 0;
}
int ghostos_confidential_validate(const ghostos_confidential_authority *authority,
    const ghostos_confidential_capability *records, size_t count,
    const ghostos_confidential_capability *capability, uint64_t subject, uint8_t required, uint64_t now_us) {
    bool present = false;
    uint8_t expected[32];
    size_t i;
    for (i = 0; i < count; ++i) if (records[i].occupied && same_capability(&records[i], capability)) present = true;
    if (capability->issuer != authority->issuer || capability->epoch != authority->epoch ||
        capability->subject != subject || now_us >= capability->expires_at_us ||
        !contains_rights(capability->rights, required) || !present)
        return now_us >= capability->expires_at_us ? 4 : 3;
    digest(capability, expected);
    for (i = 0; i < 32; ++i) if (expected[i] != capability->token[i]) return 5;
    return 0;
}
int ghostos_confidential_revoke(ghostos_confidential_capability *records, size_t count,
    const ghostos_confidential_capability *capability) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (records[i].occupied && same_capability(&records[i], capability)) {
            records[i].occupied = false;
            return 0;
        }
    }
    return 6;
}
uint64_t ghostos_confidential_revoke_all(ghostos_confidential_authority *authority,
    ghostos_confidential_capability *records, size_t count) {
    uint64_t next = authority->epoch + 1;
    size_t i;
    authority->epoch = next ? next : 1;
    for (i = 0; i < count; ++i) records[i].occupied = false;
    return authority->epoch;
}
