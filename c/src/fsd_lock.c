#include "ghostos/fsd_lock.h"
static bool same_path(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static bool ranges_overlap(bool left_whole, uint64_t left_record, bool right_whole, uint64_t right_record) {
    return left_whole || right_whole || left_record == right_record;
}
static bool conflict(const ghostos_fsd_lock_slot *lock, uint64_t owner, const uint8_t *path, size_t path_length, bool whole,
    uint64_t record, uint8_t mode) {
    return lock->occupied && lock->owner != owner && same_path(lock->path, lock->path_length, path, path_length) &&
        ranges_overlap(lock->whole, lock->record, whole, record) && (lock->mode == 1 || mode == 1);
}
int ghostos_fsd_lock(ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, const uint8_t *path, size_t path_length,
    bool whole, uint64_t record, uint8_t mode, uint64_t *handle) {
    size_t i, slot = 0;
    bool found = false;
    uint32_t generation;
    if (!path_length || path_length > GHOSTOS_FSD_PATH || mode > 1) return 4;
    for (i = 0; i < capacity; ++i) if (conflict(&locks[i], owner, path, path_length, whole, record, mode)) return 1;
    for (i = 0; i < capacity; ++i) if (!locks[i].occupied) { slot = i; found = true; break; }
    if (!found) return 2;
    generation = locks[slot].generation + 1;
    if (!generation) generation = 1;
    locks[slot].occupied = true;
    locks[slot].whole = whole;
    locks[slot].mode = mode;
    locks[slot].generation = generation;
    locks[slot].owner = owner;
    locks[slot].record = record;
    locks[slot].path_length = (uint8_t)path_length;
    for (i = 0; i < path_length; ++i) locks[slot].path[i] = path[i];
    *handle = ((uint64_t)generation << 32) | slot;
    return 0;
}
int ghostos_fsd_unlock(ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, uint64_t handle) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t slot = (uint32_t)handle;
    if (!generation || slot >= capacity || !locks[slot].occupied || locks[slot].generation != generation || locks[slot].owner != owner)
        return 3;
    locks[slot].occupied = false;
    return 0;
}
int ghostos_fsd_io(const ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, const uint8_t *path, size_t path_length,
    uint64_t record, uint8_t mode) {
    size_t i;
    if (!path_length || path_length > GHOSTOS_FSD_PATH || mode > 1) return 4;
    for (i = 0; i < capacity; ++i) if (conflict(&locks[i], owner, path, path_length, false, record, mode)) return 1;
    return 0;
}
