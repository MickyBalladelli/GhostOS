#include "ghostos/fsd_snapshot.h"
#include "ghostos/fsd_list.h"
static int snapshot_index(const ghostos_fsd_snapshot *snapshots, size_t capacity, uint64_t owner, uint64_t handle, size_t *index) {
    uint32_t generation = (uint32_t)(handle >> 32);
    uint32_t raw = (uint32_t)handle;
    if (!raw) return 2;
    *index = (size_t)raw - 1;
    if (*index >= capacity) return 2;
    if (!snapshots[*index].occupied || snapshots[*index].generation != generation || snapshots[*index].owner != owner) return 1;
    return 0;
}
static int prefix_ok(const uint8_t *buffer, size_t buffer_length, uint64_t path_length) {
    if (!path_length || path_length > GHOSTOS_FSD_SNAPSHOT_PATH || buffer_length <= GHOSTOS_FSD_SNAPSHOT_PATH) return 3;
    if (path_length == 1 && buffer[0] == '/') return 0;
    if (!buffer[0] || buffer[0] != '/') return 3;
    return 0;
}
int ghostos_fsd_snapshot_create(uint16_t rights, uint64_t owner, ghostos_fsd_snapshot *snapshots, size_t capacity,
    const uint8_t *const *names, const uint8_t *name_lengths, size_t name_count, uint64_t *handle) {
    size_t slot = 0, i;
    uint32_t generation;
    bool found = false;
    if ((rights & 8u) == 0) return 5;
    for (i = 0; i < capacity; ++i) if (!snapshots[i].occupied) { slot = i; found = true; break; }
    if (!found) return 4;
    generation = snapshots[slot].generation + 1;
    if (!generation) generation = 1;
    snapshots[slot].occupied = true;
    snapshots[slot].generation = generation;
    snapshots[slot].owner = owner;
    snapshots[slot].names = names;
    snapshots[slot].name_lengths = name_lengths;
    snapshots[slot].name_count = name_count;
    *handle = ((uint64_t)generation << 32) | (slot + 1);
    return 0;
}
int ghostos_fsd_snapshot_list(const ghostos_fsd_snapshot *snapshots, size_t capacity, uint64_t owner, uint64_t handle,
    const uint8_t *buffer, size_t buffer_length, uint64_t path_length, size_t skip, uint8_t *output, size_t output_capacity,
    size_t *written, size_t *next) {
    size_t index = 0;
    int status = prefix_ok(buffer, buffer_length, path_length);
    if (status) return status;
    status = snapshot_index(snapshots, capacity, owner, handle, &index);
    if (status) return status;
    return ghostos_fsd_list(1, snapshots[index].names, snapshots[index].name_lengths, snapshots[index].name_count, skip,
        output, output_capacity, written, next);
}
