#ifndef GHOSTOS_VOLUME_MUTATION_H
#define GHOSTOS_VOLUME_MUTATION_H
#include "ghostos/volume_tree.h"
#include "ghostos/volume_reader.h"
#include "ghostos/volume_quota.h"
/* Serialized C empty-write backend. records[0..record_count] must be the
   complete active root's decoded records, including retained/deleted versions.
   The cache is caller-owned and must match root; it is not a checkpoint cache.
   Raw tree blocks, record/cache arrays, and scratch buffers must not overlap.
   Successful mutation updates raw tree nodes, root, records, generation, and
   reader views together. Existing nodes/data stay allocated for old roots.
   This publishes to the in-memory volume; it does not flush the disk banks.
   Results: 0 success; 1..9 volume_range.h; 10 quota exceeded,
   11 version/object overflow, 12 arena full, 13 scratch/cache too small,
   14 invalid backend configuration. Failure may consume an object ID and
   allocate unreachable nodes, but does not publish a new root or generation. */
enum {
    GHOSTOS_VOLUME_MUTATION_QUOTA = 10,
    GHOSTOS_VOLUME_MUTATION_OVERFLOW = 11,
    GHOSTOS_VOLUME_MUTATION_ARENA_FULL = 12,
    GHOSTOS_VOLUME_MUTATION_SCRATCH = 13,
    GHOSTOS_VOLUME_MUTATION_INVALID = 14
};
typedef struct {
    ghostos_volume_tree tree;
    uint32_t root;
    uint64_t generation, next_object;
    ghostos_volume_quota limits;
    ghostos_volume_record *records;
    size_t record_count, record_capacity;
    ghostos_volume_quota_record *quota_records;
    size_t quota_capacity;
    ghostos_volume_range_file *files;
    size_t file_capacity;
    ghostos_volume_range_block *blocks;
    size_t block_capacity;
} ghostos_volume_mutation;
/* Validate configured storage and build initial reader views without mutation. */
int ghostos_volume_mutation_init(ghostos_volume_mutation *volume, ghostos_volume_reader *reader);
int ghostos_volume_mutation_write_empty(ghostos_volume_mutation *volume,
    ghostos_volume_reader *reader, const uint8_t *path, size_t path_length);
#endif
