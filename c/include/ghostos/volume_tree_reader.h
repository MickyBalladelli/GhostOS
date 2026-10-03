#ifndef GHOSTOS_VOLUME_TREE_READER_H
#define GHOSTOS_VOLUME_TREE_READER_H
#include "ghostos/volume_tree.h"
#include "ghostos/volume_reader.h"
#include "ghostos/volume_snapshot.h"
/* Caller-owned materialization buffers. Use separate buffers for live and
   checkpoint readers. Buffers must not overlap each other or raw storage.
   pending and visited each have at least tree.block_count entries. Trees are
   walked in child order, retaining deleted and historical records.
   Results: 0 success, 1 checkpoint missing, 5 corrupt/invalid, 6 capacity.
   Reader and record_count change only on success; scratch/cache arrays may
   change on failure. Never reuse arrays backing an existing reader.
   A checkpoint must stay pinned while its reader is in use. */
typedef struct {
    ghostos_volume_record *records;
    size_t record_capacity;
    ghostos_volume_range_file *files;
    size_t file_capacity;
    ghostos_volume_range_block *blocks;
    size_t block_capacity;
    uint32_t *pending;
    bool *visited;
    size_t traversal_capacity;
} ghostos_volume_tree_reader_buffers;
int ghostos_volume_tree_reader_init(const ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_tree_reader_buffers *buffers,
    ghostos_volume_reader *reader, size_t *record_count);
int ghostos_volume_tree_checkpoint_reader(const ghostos_volume_tree *tree,
    const ghostos_volume_pin_slot *pins, size_t pin_count, uint64_t checkpoint,
    const ghostos_volume_tree_reader_buffers *buffers,
    ghostos_volume_reader *reader, size_t *record_count);
#endif
