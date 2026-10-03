#include "ghostos/config_signature.h"
#include <assert.h>
static void trusted_key_rejects_the_wrong_node_before_an_unknown_signer(void) {
    uint8_t keys[2][32] = {0};
    bool occupied[2] = {false, false};
    uint8_t key[32], other[32], digest[32], id[16], signature[32];
    uint32_t nodes[2] = {1, 2};
    size_t i;
    for (i = 0; i < 32; ++i) { key[i] = 7; other[i] = 8; digest[i] = 1; }
    assert(!ghostos_config_trust(keys, occupied, 2, key, id));
    assert(ghostos_config_trust(keys, occupied, 2, key, id) == 1);
    assert(!ghostos_config_sign(key, digest, 1, 1, true, signature));
    assert(!ghostos_config_verify(keys, occupied, 2, id, signature, digest, 1, 1, true, 1));
    assert(ghostos_config_verify(keys, occupied, 2, id, signature, digest, 1, 1, true, 2) == 1);
    assert(ghostos_config_verify(keys, occupied, 2, other, signature, digest, 1, 1, true, 1) == 2);
    signature[0] ^= 1;
    assert(ghostos_config_verify(keys, occupied, 2, id, signature, digest, 1, 1, true, 1) == 3);
    signature[0] ^= 1;
    assert(ghostos_config_verify_cluster(keys, occupied, 2, id, signature, digest, 1, 1, true, nodes, 0) == 1);
    nodes[1] = 1;
    assert(!ghostos_config_verify_cluster(keys, occupied, 2, id, signature, digest, 1, 1, true, nodes, 2));
}
int main(void) {
    trusted_key_rejects_the_wrong_node_before_an_unknown_signer();
    return 0;
}
