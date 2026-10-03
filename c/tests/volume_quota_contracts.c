#include "ghostos/volume_quota.h"
#include <assert.h>
#include <string.h>
static void two_names_hold_twelve_bytes(void) {
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    ghostos_volume_quota_record records[3];
    ghostos_volume_quota limits = {UINT64_MAX, UINT64_MAX, UINT64_MAX};
    ghostos_volume_quota requested = {12, 2, 4};
    uint64_t retained = 0, files = 0, versions = 0;
    memset(records, 0, sizeof records);
    records[0].name = log;
    records[0].name_length = sizeof log;
    records[0].size = 3;
    records[0].occupied = true;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].size = 5;
    records[1].occupied = true;
    records[2].name = alias;
    records[2].name_length = sizeof alias;
    records[2].size = 4;
    records[2].occupied = true;
    ghostos_volume_quota_usage(records, 3, &retained, &files, &versions);
    assert(retained == 12 && files == 2 && versions == 3);
    assert(ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, 0, false));
    requested.max_blocks = 5;
    assert(!ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, 0, false));
    assert(ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, 1, false));
    assert(ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, 0, true));
    requested.max_files = 3;
    assert(!ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, 0, true));
    requested.max_bytes = UINT64_MAX;
    assert(!ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, UINT64_MAX, false));
    requested.max_bytes = UINT64_MAX - 1;
    assert(ghostos_volume_quota_enforce(records, 3, &requested, 4, 8, UINT64_MAX, false));
    requested.max_blocks = 9;
    assert(ghostos_volume_quota_set(records, 3, &limits, 4, 8, &requested));
    assert(limits.max_blocks == UINT64_MAX);
    requested.max_blocks = 4;
    requested.max_bytes = 11;
    requested.max_files = 2;
    assert(ghostos_volume_quota_set(records, 3, &limits, 4, 8, &requested));
    requested.max_bytes = 12;
    assert(!ghostos_volume_quota_set(records, 3, &limits, 4, 8, &requested));
    assert(limits.max_bytes == 12 && limits.max_files == 2 && limits.max_blocks == 4);
    assert(ghostos_volume_quota_enforce(records, 3, &limits, 4, 8, 0, false));
}
int main(void) {
    two_names_hold_twelve_bytes();
    return 0;
}
