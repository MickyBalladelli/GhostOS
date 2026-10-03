#include "ghostos/fsd_lock.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void whole_file_lock_blocks_until_unlock_and_a_record_lock_is_local(void) {
    const char *locked = "/data/locked";
    const char *records = "/data/records";
    ghostos_fsd_lock_slot locks[2];
    uint64_t handle = 0;
    memset(locks, 0, sizeof locks);
    assert(!ghostos_fsd_lock(locks, 2, 1, (const uint8_t *)locked, text_length(locked), true, 0, 1, &handle));
    assert(ghostos_fsd_io(locks, 2, 8, (const uint8_t *)locked, text_length(locked), 0, 0) == 1);
    assert(!ghostos_fsd_unlock(locks, 2, 1, handle));
    assert(!ghostos_fsd_io(locks, 2, 8, (const uint8_t *)locked, text_length(locked), 0, 0));
    assert(!ghostos_fsd_lock(locks, 2, 1, (const uint8_t *)records, text_length(records), false, 4, 1, &handle));
    assert(!ghostos_fsd_io(locks, 2, 8, (const uint8_t *)records, text_length(records), 0, 0));
    assert(ghostos_fsd_io(locks, 2, 8, (const uint8_t *)records, text_length(records), 4, 0) == 1);
    assert(!ghostos_fsd_unlock(locks, 2, 1, handle));
    assert(!ghostos_fsd_io(locks, 2, 8, (const uint8_t *)records, text_length(records), 4, 0));
}
int main(void) {
    whole_file_lock_blocks_until_unlock_and_a_record_lock_is_local();
    return 0;
}
