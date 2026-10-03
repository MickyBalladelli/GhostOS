#include "ghostos/volume_quota.h"
static int compare_name(const ghostos_volume_quota_record *left, const ghostos_volume_quota_record *right) {
    size_t i, length = left->name_length < right->name_length ? left->name_length : right->name_length;
    for (i = 0; i < length; ++i) {
        if (left->name[i] < right->name[i]) return -1;
        if (left->name[i] > right->name[i]) return 1;
    }
    if (left->name_length < right->name_length) return -1;
    if (left->name_length > right->name_length) return 1;
    return 0;
}
static uint64_t saturating_add(uint64_t value, uint64_t extra) {
    if (value > UINT64_MAX - extra) return UINT64_MAX;
    return value + extra;
}
void ghostos_volume_quota_usage(const ghostos_volume_quota_record *records, size_t count, uint64_t *retained_bytes, uint64_t *file_count, uint64_t *retained_versions) {
    bool have_previous = false;
    int previous = -1;
    *retained_bytes = 0;
    *file_count = 0;
    *retained_versions = 0;
    for (;;) {
        size_t i;
        int best = -1;
        for (i = 0; i < count; ++i) {
            if (!records[i].occupied) continue;
            if (have_previous && compare_name(&records[i], &records[previous]) < 0) continue;
            if (have_previous && compare_name(&records[i], &records[previous]) == 0 && i <= (size_t)previous) continue;
            if (best >= 0 && compare_name(&records[i], &records[best]) > 0) continue;
            if (best >= 0 && compare_name(&records[i], &records[best]) == 0 && i >= (size_t)best) continue;
            best = (int)i;
        }
        if (best < 0) return;
        if (!have_previous || compare_name(&records[best], &records[previous]) != 0) *file_count = saturating_add(*file_count, 1);
        *retained_bytes = saturating_add(*retained_bytes, records[best].size);
        *retained_versions = saturating_add(*retained_versions, 1);
        previous = best;
        have_previous = true;
    }
}
int ghostos_volume_quota_enforce(const ghostos_volume_quota_record *records, size_t count, const ghostos_volume_quota *limits, uint64_t used_blocks, uint64_t capacity_blocks, uint64_t additional_bytes, bool new_file) {
    uint64_t retained = 0, files = 0, versions = 0;
    if (used_blocks >= limits->max_blocks || used_blocks >= capacity_blocks) return 1;
    ghostos_volume_quota_usage(records, count, &retained, &files, &versions);
    if (saturating_add(retained, additional_bytes) > limits->max_bytes) return 1;
    if (new_file && files >= limits->max_files) return 1;
    return 0;
}
int ghostos_volume_quota_set(const ghostos_volume_quota_record *records, size_t count, ghostos_volume_quota *limits, uint64_t used_blocks, uint64_t capacity_blocks, const ghostos_volume_quota *requested) {
    uint64_t retained = 0, files = 0, versions = 0;
    if (requested->max_blocks != UINT64_MAX && requested->max_blocks > capacity_blocks) return 1;
    ghostos_volume_quota_usage(records, count, &retained, &files, &versions);
    if (retained > requested->max_bytes || files > requested->max_files || used_blocks > requested->max_blocks) return 1;
    *limits = *requested;
    return 0;
}
