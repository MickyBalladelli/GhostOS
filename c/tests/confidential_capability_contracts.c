#include "ghostos/confidential_capability.h"
#include <assert.h>
static void issued_capability_expires_before_other_failures(void) {
    ghostos_confidential_authority authority;
    ghostos_confidential_capability records[2] = {0};
    uint8_t attestation[32];
    size_t index = 9;
    size_t i;
    for (i = 0; i < 32; ++i) attestation[i] = 9;
    assert(!ghostos_confidential_authority_init(&authority, 55));
    assert(ghostos_confidential_issue(&authority, records, 2, 2, 0, 1, 16384, 4096, 1, 100, 3, attestation, &index) == 1);
    assert(!ghostos_confidential_issue(&authority, records, 2, 2, 10, 1, 16384, 4096, 1, 100, 3, attestation, &index));
    assert(index == 0 && records[0].id == 1 && authority.next_id == 2);
    assert(!ghostos_confidential_validate(&authority, records, 2, &records[0], 10, 1, 4));
    assert(ghostos_confidential_validate(&authority, records, 2, &records[0], 11, 1, 4) == 3);
    assert(ghostos_confidential_validate(&authority, records, 2, &records[0], 11, 1, 100) == 4);
    assert(ghostos_confidential_revoke_all(&authority, records, 2) == 2);
    assert(ghostos_confidential_validate(&authority, records, 2, &records[0], 10, 1, 4) == 3);
}
int main(void) {
    issued_capability_expires_before_other_failures();
    return 0;
}
