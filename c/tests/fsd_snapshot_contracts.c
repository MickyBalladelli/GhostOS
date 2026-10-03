#include "ghostos/fsd_snapshot.h"
#include <assert.h>
#include <string.h>
static void snapshot_list_rejects_a_missing_path_length(void) {
    const uint8_t name[] = {'d', 'a', 't', 'a'};
    const uint8_t *names[] = {name};
    const uint8_t lengths[] = {4};
    ghostos_fsd_snapshot snapshots[1];
    uint8_t buffer[GHOSTOS_FSD_SNAPSHOT_PATH + 28];
    uint8_t output[28];
    uint64_t handle = 0;
    size_t written = 0, next = 0;
    memset(snapshots, 0, sizeof snapshots);
    memset(buffer, 0, sizeof buffer);
    buffer[0] = '/';
    assert(ghostos_fsd_snapshot_create(0, 1, snapshots, 1, names, lengths, 1, &handle) == 5);
    assert(!ghostos_fsd_snapshot_create(8, 1, snapshots, 1, names, lengths, 1, &handle));
    assert(ghostos_fsd_snapshot_list(snapshots, 1, 1, handle, buffer, sizeof buffer, 0, 0, output, sizeof output, &written, &next) == 3);
    assert(buffer[0] == '/');
    assert(!ghostos_fsd_snapshot_list(snapshots, 1, 1, handle, buffer, sizeof buffer, 1, 0, output, sizeof output, &written, &next));
    assert(written >= 22 && buffer[0] == '/' && !memcmp(output + 22, "data", 4));
    assert(ghostos_fsd_snapshot_list(snapshots, 1, 8, handle, buffer, sizeof buffer, 1, 0, output, sizeof output, &written, &next) == 1);
}
int main(void) {
    snapshot_list_rejects_a_missing_path_length();
    return 0;
}
