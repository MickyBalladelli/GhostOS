#include "ghostos/volume_snapshot.h"
#include <assert.h>
#include <string.h>
static void pinned_log_stays_old_after_the_live_write(void) {
    const uint8_t name[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t old_bytes[] = {'o', 'l', 'd'};
    const uint8_t new_bytes[] = {'n', 'e', 'w'};
    ghostos_volume_file files[2];
    ghostos_volume_pin_slot pins[1];
    uint8_t output[3];
    uint64_t next_id = 1, id = 0;
    size_t read = 0;
    memset(pins, 0, sizeof pins);
    files[0].name = name;
    files[0].name_length = sizeof name;
    files[0].data_length = 3;
    files[0].version = 1;
    files[0].generation = 1;
    files[0].data = old_bytes;
    files[0].deleted = false;
    files[1] = files[0];
    files[1].version = 2;
    files[1].generation = 2;
    files[1].data = new_bytes;
    assert(!ghostos_volume_pin(pins, 1, &next_id, 1, &id));
    assert(id == 1 && next_id == 2);
    assert(ghostos_volume_pin(pins, 1, &next_id, 1, &id) == 1);
    assert(!ghostos_volume_read(files, 2, name, sizeof name, output, sizeof output, &read));
    assert(read == 3 && !memcmp(output, "new", 3));
    assert(!ghostos_volume_snapshot_read(pins, 1, files, 2, 1, name, sizeof name, output, sizeof output, &read));
    assert(!memcmp(output, "old", 3));
    assert(!ghostos_volume_unpin(pins, 1, 1));
    assert(ghostos_volume_snapshot_read(pins, 1, files, 2, 1, name, sizeof name, output, sizeof output, &read) == 3);
}
int main(void) {
    pinned_log_stays_old_after_the_live_write();
    return 0;
}
