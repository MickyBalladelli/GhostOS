#include "ghostos/volume_retention.h"
#include <assert.h>
static bool live_version(const ghostos_volume_version *versions, size_t count, const uint8_t *name, uint8_t name_length, uint32_t version) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (versions[i].deleted || versions[i].name_length != name_length || versions[i].version != version) continue;
        return true;
    }
    return false;
}
static void purge_removes_the_oldest_log_version(void) {
    const uint8_t name[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    ghostos_volume_version versions[3];
    uint64_t generation = 4;
    size_t purged = 9, i;
    for (i = 0; i < 3; ++i) {
        versions[i].name = name;
        versions[i].name_length = sizeof name;
        versions[i].version = (uint32_t)(i + 1);
        versions[i].deleted = false;
    }
    assert(ghostos_volume_purge(versions, 2, name, sizeof name, 0, 8, &generation, &purged) == 1);
    assert(!ghostos_volume_purge(versions, 2, name, sizeof name, 1, 8, &generation, &purged));
    assert(purged == 1 && generation == 5 && versions[0].deleted && !versions[1].deleted);
    assert(!live_version(versions, 2, name, sizeof name, 1));
    assert(!ghostos_volume_purge(versions, 2, name, sizeof name, 1, 8, &generation, &purged));
    assert(!purged && generation == 5);
    versions[0].deleted = false;
    assert(!ghostos_volume_purge(versions, 3, name, sizeof name, 1, 1, &generation, &purged));
    assert(purged == 1 && versions[0].deleted && !versions[1].deleted && !versions[2].deleted);
}
int main(void) {
    purge_removes_the_oldest_log_version();
    return 0;
}
