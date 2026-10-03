#include "ghostos/volume_storage.h"
#include "ghostos/volume_superblock.h"
#include "ghostos/volume_type_map.h"
#include "ghostos/volume_validate.h"
int ghostos_volume_storage_flush(const ghostos_volume_mutation *mutation,
    ghostos_volume *banks, uint64_t next_checkpoint,
    const ghostos_volume_storage_io *io, uint8_t *map, size_t map_capacity,
    uint8_t *header, size_t header_capacity, uint64_t *sequence) {
    ghostos_volume_checkpoint checkpoints[16];
    size_t i, count = 0;
    uint64_t checksum, next, base;
    uint8_t bank;
    int validation;
    const ghostos_volume_tree *tree;
    if (!mutation || !banks || !io || !io->write_block || !io->flush || !sequence ||
        banks->active > 1 || !next_checkpoint || !mutation->next_object ||
        (mutation->root && !mutation->generation) ||
        (mutation->pin_capacity && !mutation->pins)) return 1;
    tree = &mutation->tree;
    if (tree->block_count > (GHOSTOS_VOLUME_TYPE_MAP - 8u) * 4u ||
        mutation->root > tree->block_count ||
        (tree->block_count && (!tree->blocks || !tree->kinds))) return 1;
    if (!map || !header || map_capacity < GHOSTOS_VOLUME_TYPE_MAP ||
        header_capacity < GHOSTOS_VOLUME_SUPERBLOCK || !tree->payload ||
        tree->payload_capacity < GHOSTOS_VOLUME_DATA) return 5;
    if (banks->sequence == UINT64_MAX) return 2;
    validation = ghostos_volume_validate(mutation, next_checkpoint);
    if (validation) return validation;
    next = banks->sequence + 1;
    bank = (uint8_t)(1u - banks->active);
    base = (uint64_t)bank * (tree->block_count + 2u);
    for (i = 0; i < mutation->pin_capacity; ++i) {
        const ghostos_volume_pin_slot *pin = &mutation->pins[i];
        if (!pin->occupied) continue;
        if (count == 16) return 1;
        checkpoints[count++] = (ghostos_volume_checkpoint){pin->id, pin->generation, pin->root};
    }
    if (ghostos_volume_type_map_encode(tree->kinds, tree->block_count, map, map_capacity, &checksum) ||
        ghostos_volume_superblock_encode(4, next, mutation->generation, mutation->root,
            next_checkpoint, mutation->next_object, checksum, tree->block_count, header, header_capacity) ||
        ghostos_volume_superblock_set_checkpoints(header, header_capacity, tree->block_count, checkpoints, count) ||
        ghostos_volume_superblock_set_limits(header, header_capacity, tree->block_count,
            mutation->limits.max_bytes, mutation->limits.max_files, mutation->limits.max_blocks)) return 1;
    /* Validate serialized blocks before issuing any writes. */
    for (i = 0; i < tree->block_count; ++i) {
        const uint8_t *raw = tree->blocks + i * GHOSTOS_VOLUME_BLOCK;
        if (tree->kinds[i] == 1 || tree->kinds[i] == 2) {
            ghostos_volume_tree_node node;
            if (ghostos_volume_tree_decode(raw, GHOSTOS_VOLUME_BLOCK, tree->payload,
                tree->payload_capacity, &node) || node.kind != tree->kinds[i]) return 1;
        } else if (tree->kinds[i] == 3) {
            uint32_t successor;
            size_t length;
            if (ghostos_volume_decode_data(raw, GHOSTOS_VOLUME_BLOCK, &successor,
                tree->payload, tree->payload_capacity, &length)) return 1;
        }
    }
    if (io->write_block(io->context, base + 1, map)) return 3;
    for (i = 0; i < tree->block_count; ++i) {
        const uint8_t *raw = tree->blocks + i * GHOSTOS_VOLUME_BLOCK;
        if (!tree->kinds[i]) {
            size_t byte;
            for (byte = 0; byte < GHOSTOS_VOLUME_TYPE_MAP; ++byte) map[byte] = 0;
            raw = map;
        }
        if (io->write_block(io->context, base + 2 + i, raw)) return 3;
    }
    if (io->write_block(io->context, base, header) || io->flush(io->context)) return 3;
    banks->banks[bank] = (ghostos_volume_bank){true, next, mutation->generation, 4};
    banks->active = bank;
    banks->sequence = next;
    *sequence = next;
    if (io->interrupted && io->interrupted(io->context)) return 4;
    return 0;
}
