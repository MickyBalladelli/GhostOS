#include "ghostos/inference_journal.h"
#include <assert.h>
#include <string.h>
static void checkpoint_ack_and_failover_follow_the_surviving_journal(void) {
    ghostos_inference_journal entries[2];
    ghostos_inference_checkpoint initial, checkpoint, repair;
    uint64_t handle = 0, rng = 0;
    uint32_t journal = 0;
    uint8_t state = 0;
    uint8_t corrupt[GHOSTOS_INFERENCE_RECOVERY_BYTES];
    size_t i;
    memset(entries, 0, sizeof entries);
    assert(!ghostos_inference_journal_begin(entries, 2, 9, (1ull << 32) | 1, (1ull << 32) | 2, 2, 3, 44, &handle, &initial));
    assert(!ghostos_inference_recovery_decode(initial.bytes, GHOSTOS_INFERENCE_RECOVERY_BYTES, &rng));
    assert(rng == 44);
    assert(ghostos_inference_journal_acknowledge(entries, 2, handle, initial.epoch, true, false, &state) == 1);
    assert(!ghostos_inference_journal_acknowledge(entries, 2, handle, initial.epoch, true, true, &state));
    assert(!ghostos_inference_journal_prepare(entries, 2, handle, 5, 55, &checkpoint));
    assert(!ghostos_inference_journal_acknowledge(entries, 2, handle, checkpoint.epoch, true, true, &state));
    assert(state == 1);
    assert(!ghostos_inference_journal_fail(entries, 2, handle, 2, &journal));
    assert(journal == 3);
    assert(!ghostos_inference_journal_repair(entries, 2, handle, 4, &repair));
    assert(repair.primary == 3 && repair.replica == 4);
    for (i = 0; i < GHOSTOS_INFERENCE_RECOVERY_BYTES; ++i) corrupt[i] = checkpoint.bytes[i];
    corrupt[64] ^= 1;
    assert(ghostos_inference_recovery_decode(corrupt, GHOSTOS_INFERENCE_RECOVERY_BYTES, &rng) == 6);
}
int main(void) {
    checkpoint_ack_and_failover_follow_the_surviving_journal();
    return 0;
}
