#include "ghostos/attestation.h"
#include <assert.h>
static void quote_admission_requires_a_live_challenge(void) {
    ghostos_attestation_node nodes[2] = {0};
    uint8_t measurement[32], key[32], nonce[32], signature[32];
    size_t index = 9;
    size_t i;
    for (i = 0; i < 32; ++i) { measurement[i] = 7; key[i] = 9; nonce[i] = 3; }
    assert(!ghostos_attestation_register(nodes, 2, 2, 3, measurement, key, &index));
    assert(index == 0);
    assert(ghostos_attestation_register(nodes, 2, 2, 3, measurement, key, &index) == 2);
    assert(ghostos_attestation_admit(nodes, 2, 2, 3, 2, nonce, measurement, signature, 3) == 5);
    assert(!ghostos_attestation_challenge(nodes, 2, 2, nonce, 50));
    assert(!ghostos_attestation_quote(2, 3, 2, nonce, measurement, key, signature));
    assert(!ghostos_attestation_admit(nodes, 2, 2, 3, 2, nonce, measurement, signature, 3));
    assert(ghostos_attestation_is_admitted(nodes, 2, 2));
    assert(!ghostos_attestation_challenge(nodes, 2, 2, nonce, 50));
    assert(ghostos_attestation_admit(nodes, 2, 2, 3, 2, nonce, measurement, signature, 51) == 6);
    assert(!ghostos_attestation_challenge(nodes, 2, 2, nonce, 50));
    signature[0] ^= 1;
    assert(ghostos_attestation_admit(nodes, 2, 2, 3, 2, nonce, measurement, signature, 3) == 7);
}
int main(void) {
    quote_admission_requires_a_live_challenge();
    return 0;
}
