#include "ghostos/attestation.h"
#include "ghostos/vm_snapshot_auth.h"
static bool zero32(const uint8_t bytes[32]) {
    size_t i;
    for (i = 0; i < 32; ++i) if (bytes[i]) return false;
    return true;
}
static bool same32(const uint8_t *left, const uint8_t *right) {
    size_t i;
    for (i = 0; i < 32; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int find_node(ghostos_attestation_node *nodes, size_t count, uint32_t node) {
    size_t i;
    for (i = 0; i < count; ++i) if (nodes[i].occupied && nodes[i].node == node) return (int)i;
    return -1;
}
static void quote_material(uint32_t node, uint8_t root, uint64_t issued_at_us, const uint8_t nonce[32],
    const uint8_t measurement[32], uint8_t material[77]) {
    size_t i;
    material[0] = (uint8_t)(node >> 24);
    material[1] = (uint8_t)(node >> 16);
    material[2] = (uint8_t)(node >> 8);
    material[3] = (uint8_t)node;
    material[4] = root;
    for (i = 0; i < 8; ++i) material[5 + i] = (uint8_t)(issued_at_us >> (56 - 8 * i));
    for (i = 0; i < 32; ++i) {
        material[13 + i] = nonce[i];
        material[45 + i] = measurement[i];
    }
}
int ghostos_attestation_register(ghostos_attestation_node *nodes, size_t count, uint32_t node,
    uint8_t root, const uint8_t measurement[32], const uint8_t key[32], size_t *index) {
    size_t free_slot = count;
    size_t i;
    if (!root || root > 5 || zero32(measurement)) return 1;
    for (i = 0; i < count; ++i) {
        if (!nodes[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (nodes[i].node == node) return 2;
    }
    if (free_slot == count) return 3;
    nodes[free_slot] = (ghostos_attestation_node){0};
    nodes[free_slot].node = node;
    nodes[free_slot].root = root;
    for (i = 0; i < 32; ++i) {
        nodes[free_slot].measurement[i] = measurement[i];
        nodes[free_slot].key[i] = key[i];
    }
    nodes[free_slot].occupied = true;
    *index = free_slot;
    return 0;
}
int ghostos_attestation_rotate_key(ghostos_attestation_node *nodes, size_t count, uint32_t node, const uint8_t key[32]) {
    int index = find_node(nodes, count, node);
    size_t i;
    if (index < 0) return 4;
    for (i = 0; i < 32; ++i) nodes[index].key[i] = key[i];
    nodes[index].has_challenge = false;
    nodes[index].admitted = false;
    return 0;
}
int ghostos_attestation_revoke(ghostos_attestation_node *nodes, size_t count, uint32_t node) {
    int index = find_node(nodes, count, node);
    if (index < 0) return 4;
    nodes[index].occupied = false;
    return 0;
}
int ghostos_attestation_challenge(ghostos_attestation_node *nodes, size_t count, uint32_t node,
    const uint8_t nonce[32], uint64_t expires_at_us) {
    int index = find_node(nodes, count, node);
    size_t i;
    if (zero32(nonce) || !expires_at_us) return 1;
    if (index < 0) return 4;
    for (i = 0; i < 32; ++i) nodes[index].nonce[i] = nonce[i];
    nodes[index].expires_at_us = expires_at_us;
    nodes[index].has_challenge = true;
    nodes[index].admitted = false;
    return 0;
}
int ghostos_attestation_quote(uint32_t node, uint8_t root, uint64_t issued_at_us, const uint8_t nonce[32],
    const uint8_t measurement[32], const uint8_t key[32], uint8_t signature[32]) {
    uint8_t material[77];
    ghostos_vm_snapshot_auth_part part;
    if (zero32(nonce) || zero32(measurement)) return 1;
    quote_material(node, root, issued_at_us, nonce, measurement, material);
    part.bytes = material;
    part.length = sizeof material;
    ghostos_vm_snapshot_hmac_sha256(key, &part, 1, signature);
    return 0;
}
int ghostos_attestation_admit(ghostos_attestation_node *nodes, size_t count, uint32_t node, uint8_t root,
    uint64_t issued_at_us, const uint8_t nonce[32], const uint8_t measurement[32], const uint8_t signature[32], uint64_t now_us) {
    int index = find_node(nodes, count, node);
    uint8_t expected[32];
    if (index < 0 || !nodes[index].has_challenge) return 5;
    if (now_us > nodes[index].expires_at_us || issued_at_us > now_us) return 6;
    if (root != nodes[index].root || !same32(nonce, nodes[index].nonce) || !same32(measurement, nodes[index].measurement))
        return 5;
    if (ghostos_attestation_quote(node, root, issued_at_us, nonce, measurement, nodes[index].key, expected) ||
        !ghostos_vm_snapshot_auth_equal(expected, signature, 32)) return 7;
    nodes[index].admitted = true;
    nodes[index].has_challenge = false;
    return 0;
}
bool ghostos_attestation_is_admitted(const ghostos_attestation_node *nodes, size_t count, uint32_t node) {
    size_t i;
    for (i = 0; i < count; ++i) if (nodes[i].occupied && nodes[i].node == node && nodes[i].admitted) return true;
    return false;
}
