#include "ghostos/remote_debug.h"
#include <assert.h>
static void remote_session_matches_target_before_authorization(void) {
    uint8_t wire[GHOSTOS_REMOTE_DEBUG_WIRE_BYTES] = {0};
    uint64_t resource = 0;
    uint64_t nonce = 0;
    wire[0] = 'S'; wire[1] = 'Y'; wire[2] = 'C'; wire[3] = 'A'; wire[4] = 1;
    wire[23] = 3;
    wire[63] = 9;
    assert(ghostos_remote_debug_open(wire, 16, 3, &resource, &nonce) == 1);
    wire[4] = 2;
    assert(ghostos_remote_debug_open(wire, sizeof wire, 3, &resource, &nonce) == 1);
    wire[4] = 1;
    assert(ghostos_remote_debug_open(wire, sizeof wire, 4, &resource, &nonce) == 2);
    wire[63] = 0;
    assert(ghostos_remote_debug_open(wire, sizeof wire, 3, &resource, &nonce) == 2);
    wire[63] = 9;
    assert(!ghostos_remote_debug_open(wire, sizeof wire, 3, &resource, &nonce));
    assert(resource == 3 && nonce == 9);
    assert(ghostos_remote_debug_rights(0) == 0x1001);
    assert(ghostos_remote_debug_rights(1) == 0x1002);
    assert(ghostos_remote_debug_rights(2) == 0x1000);
    assert(ghostos_remote_debug_permit(9, 9, true));
    assert(!ghostos_remote_debug_permit(8, 9, true));
    assert(!ghostos_remote_debug_permit(9, 9, false));
}
int main(void) {
    remote_session_matches_target_before_authorization();
    return 0;
}
