#ifndef GHOSTOS_VOLUME_QUOTA_H
#define GHOSTOS_VOLUME_QUOTA_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 accepted, 1 quota exceeded. Block use is checked before retained
   bytes, and a new file is checked last. Setting a limit stores it only when
   the current usage is not already over that limit. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint64_t size;
    bool occupied;
} ghostos_volume_quota_record;
typedef struct {
    uint64_t max_bytes, max_files, max_blocks;
} ghostos_volume_quota;
void ghostos_volume_quota_usage(const ghostos_volume_quota_record *records, size_t count, uint64_t *retained_bytes, uint64_t *file_count, uint64_t *retained_versions);
int ghostos_volume_quota_enforce(const ghostos_volume_quota_record *records, size_t count, const ghostos_volume_quota *limits, uint64_t used_blocks, uint64_t capacity_blocks, uint64_t additional_bytes, bool new_file);
int ghostos_volume_quota_set(const ghostos_volume_quota_record *records, size_t count, ghostos_volume_quota *limits, uint64_t used_blocks, uint64_t capacity_blocks, const ghostos_volume_quota *requested);
#endif
