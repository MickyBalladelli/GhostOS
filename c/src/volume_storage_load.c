#include "ghostos/volume_storage.h"
#include "ghostos/volume_superblock.h"
#include "ghostos/volume_type_map.h"
#include "ghostos/volume_tree_reader.h"
static uint64_t load_le(const uint8_t *bytes) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value |= (uint64_t)bytes[i] << (8 * i);
    return value;
}
static int candidate(ghostos_volume_mutation *destination, ghostos_volume_reader *reader,
    const uint8_t *image, uint32_t root, uint64_t generation, uint64_t *next_checkpoint) {
    ghostos_volume_mutation loaded = *destination;
    ghostos_volume_reader view;
    ghostos_volume_checkpoint checkpoints[16];
    ghostos_volume_tree_reader_buffers buffers = {
        loaded.records, loaded.record_capacity, loaded.files, loaded.file_capacity,
        loaded.blocks, loaded.block_capacity, loaded.pending, loaded.marked, loaded.gc_capacity
    };
    size_t count, i, byte;
    int status;
    loaded.root = root;
    loaded.generation = generation;
    loaded.next_object = load_le(image + 72);
    loaded.limits = (ghostos_volume_quota){load_le(image + 464), load_le(image + 472), load_le(image + 480)};
    if (ghostos_volume_superblock_checkpoints(image, GHOSTOS_VOLUME_BLOCK,
        loaded.tree.block_count, checkpoints, 16, &count)) return 1;
    if (count > loaded.pin_capacity) return 5;
    if (ghostos_volume_type_map_decode(image + GHOSTOS_VOLUME_BLOCK, GHOSTOS_VOLUME_BLOCK,
        loaded.tree.block_count, load_le(image + 64), loaded.tree.kinds, loaded.tree.block_count)) return 1;
    for (i = 0; i < loaded.tree.block_count; ++i) {
        const uint8_t *raw = image + (i + 2) * GHOSTOS_VOLUME_BLOCK;
        if (!loaded.tree.kinds[i]) {
            for (byte = 0; byte < GHOSTOS_VOLUME_BLOCK; ++byte) if (raw[byte]) return 1;
        } else if (loaded.tree.kinds[i] == 3) {
            uint32_t successor;
            size_t length;
            if (ghostos_volume_decode_data(raw, GHOSTOS_VOLUME_BLOCK, &successor,
                loaded.tree.payload, loaded.tree.payload_capacity, &length)) return 1;
        } else {
            ghostos_volume_tree_node node;
            if (ghostos_volume_tree_decode(raw, GHOSTOS_VOLUME_BLOCK, loaded.tree.payload,
                loaded.tree.payload_capacity, &node) || node.kind != loaded.tree.kinds[i]) return 1;
        }
        for (byte = 0; byte < GHOSTOS_VOLUME_BLOCK; ++byte)
            loaded.tree.blocks[i * GHOSTOS_VOLUME_BLOCK + byte] = raw[byte];
    }
    status = ghostos_volume_tree_reader_init(&loaded.tree, root, &buffers, &view, &loaded.record_count);
    if (status) return status == 6 ? 5 : 1;
    status = ghostos_volume_mutation_init(&loaded, &view);
    if (status) return status == GHOSTOS_VOLUME_MUTATION_SCRATCH ? 5 : 1;
    for (i = 0; i < loaded.pin_capacity; ++i) loaded.pins[i] = (ghostos_volume_pin_slot){0};
    for (i = 0; i < count; ++i) loaded.pins[i] = (ghostos_volume_pin_slot){
        .occupied = true, .id = checkpoints[i].id,
        .generation = checkpoints[i].generation, .root = checkpoints[i].root
    };
    *next_checkpoint = load_le(image + 48);
    *destination = loaded;
    *reader = view;
    return 0;
}
int ghostos_volume_storage_load(ghostos_volume_mutation *mutation,
    ghostos_volume_reader *reader, ghostos_volume *banks, uint64_t *next_checkpoint,
    void *context, int (*read_block)(void *context, uint64_t block, uint8_t *bytes),
    uint8_t *staging, size_t staging_capacity) {
    ghostos_volume state = {0};
    uint32_t roots[2] = {0};
    size_t blocks, stride, total, i;
    unsigned order[2] = {0, 1};
    int status;
    if (!mutation || !reader || !banks || !next_checkpoint || !read_block) return 1;
    blocks = mutation->tree.block_count;
    if (blocks > (GHOSTOS_VOLUME_TYPE_MAP - 8u) * 4u ||
        (blocks && (!mutation->tree.blocks || !mutation->tree.kinds)) ||
        (mutation->pin_capacity && !mutation->pins)) return 1;
    stride = (blocks + 2) * GHOSTOS_VOLUME_BLOCK;
    total = stride * 2;
    if (!staging || staging_capacity < total || !mutation->tree.payload ||
        mutation->tree.payload_capacity < GHOSTOS_VOLUME_DATA) return 5;
    for (i = 0; i < total / GHOSTOS_VOLUME_BLOCK; ++i)
        if (read_block(context, i, staging + i * GHOSTOS_VOLUME_BLOCK)) return 3;
    for (i = 0; i < 2; ++i) {
        uint16_t version;
        uint64_t sequence, generation;
        if (!ghostos_volume_superblock_decode(staging + i * stride, GHOSTOS_VOLUME_BLOCK,
            blocks, &version, &sequence, &generation, &roots[i]))
            state.banks[i] = (ghostos_volume_bank){true, sequence, generation, version};
    }
    if (state.banks[0].valid && state.banks[1].valid &&
        state.banks[0].sequence == state.banks[1].sequence) return 1;
    if (state.banks[1].valid && (!state.banks[0].valid ||
        state.banks[1].sequence > state.banks[0].sequence)) { order[0] = 1; order[1] = 0; }
    for (i = 0; i < 2; ++i) {
        unsigned bank = order[i];
        if (!state.banks[bank].valid) continue;
        status = candidate(mutation, reader, staging + bank * stride, roots[bank],
            state.banks[bank].generation, next_checkpoint);
        if (status == 5) return 5;
        if (status) { state.banks[bank].valid = false; continue; }
        state.active = (uint8_t)bank;
        state.sequence = state.banks[bank].sequence;
        *banks = state;
        return 0;
    }
    return 1;
}
