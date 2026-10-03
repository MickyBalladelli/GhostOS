#ifndef GHOSTOS_VOLUME_TREE_H
#define GHOSTOS_VOLUME_TREE_H
#include "ghostos/volume_record.h"
#include "ghostos/volume_block.h"
#define GHOSTOS_VOLUME_TREE_KEYS 7u
typedef struct {
    uint8_t name[GHOSTOS_VOLUME_NAME];
    uint16_t name_length;
    uint32_t version;
} ghostos_volume_tree_key;
typedef struct {
    uint8_t kind, length;
    union {
        ghostos_volume_record records[GHOSTOS_VOLUME_TREE_KEYS];
        struct {
            ghostos_volume_tree_key keys[GHOSTOS_VOLUME_TREE_KEYS];
            uint32_t children[GHOSTOS_VOLUME_TREE_KEYS + 1];
        } branch;
    } entries;
} ghostos_volume_tree_node;
typedef struct {
    ghostos_volume_tree_node node;
    size_t child;
} ghostos_volume_tree_frame;
/* Mutable raw block arena and caller-owned traversal/payload scratch.
   Kinds: empty=0, leaf=1, branch=2, data=3. IDs index blocks at ID - 1.
   Frames bound traversal depth; no recursion or heap allocation is used.
   Scratch buffers and frame arrays must not overlap the block arena or kinds.
   payload has at least GHOSTOS_VOLUME_DATA bytes. Operations are serialized.
   Results: 0 success, 1 corrupt, 2 scratch too small, 3 arena full,
   4 already exists, 5 not found. Output root changes only on success.
   Failed mutations may leave allocated unreachable blocks, as in the source
   allocator; callers collect them without reclaiming active/checkpoint roots.
   Existing nodes are never overwritten, preserving checkpoint tree roots. */
typedef struct {
    uint8_t *blocks, *kinds, *payload;
    size_t block_count, payload_capacity;
    ghostos_volume_tree_frame *frames;
    size_t frame_capacity;
} ghostos_volume_tree;
/* Node codecs retain all 192 name bytes, validate UTF-8 in the named prefix,
   and accept NUL bytes as the native disk decoder does. Tree header/payload
   reserved and trailing bytes retain the existing decoder acceptance. */
int ghostos_volume_tree_decode(const uint8_t *block, size_t capacity,
    uint8_t *payload, size_t payload_capacity, ghostos_volume_tree_node *node);
int ghostos_volume_tree_encode(const ghostos_volume_tree_node *node,
    uint8_t *payload, size_t payload_capacity, uint8_t *block, size_t capacity);
int ghostos_volume_tree_insert(ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_record *record, uint32_t *new_root);
/* Replacement searches children in order, matching the original algorithm. */
int ghostos_volume_tree_replace(ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_record *record, uint32_t *new_root);
#endif
