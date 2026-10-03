#include "ghostos/volume_retention.h"
static bool same_name(const ghostos_volume_version *version, const uint8_t *name, size_t name_length) {
    size_t i;
    if (version->name_length != name_length) return false;
    for (i = 0; i < name_length; ++i) if (version->name[i] != name[i]) return false;
    return true;
}
static void span(const ghostos_volume_version *versions, size_t count, const uint8_t *name, size_t name_length, uint32_t *live, bool *have_oldest, uint32_t *oldest, size_t *oldest_index) {
    size_t i;
    *live = 0;
    *have_oldest = false;
    for (i = 0; i < count; ++i) {
        if (versions[i].deleted || !same_name(&versions[i], name, name_length)) continue;
        *live += 1;
        if (!*have_oldest || versions[i].version < *oldest) {
            *oldest = versions[i].version;
            *oldest_index = i;
            *have_oldest = true;
        }
    }
}
int ghostos_volume_purge(ghostos_volume_version *versions, size_t count, const uint8_t *name, size_t name_length,
    uint32_t keep_latest, size_t limit, uint64_t *generation, size_t *purged) {
    *purged = 0;
    if (!keep_latest) return 1;
    while (*purged < limit) {
        uint32_t live = 0, oldest = 0;
        size_t oldest_index = 0;
        bool have_oldest = false;
        span(versions, count, name, name_length, &live, &have_oldest, &oldest, &oldest_index);
        if (live <= keep_latest) break;
        if (!have_oldest) return 1;
        versions[oldest_index].deleted = true;
        if (*generation < UINT64_MAX) *generation += 1;
        *purged += 1;
    }
    return 0;
}
