#include "ghostos/fsd_namespace.h"
static bool same_bytes(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int path_ok(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > GHOSTOS_FSD_NAMESPACE_PATH || path[0] != '/' || (length > 1 && path[length - 1] == '/')) return 4;
    for (i = 0; i < length; ++i) {
        if (!path[i] || path[i] == '\\') return 4;
        if (i + 1 < length && path[i] == '/' && path[i + 1] == '/') return 4;
    }
    i = 0;
    while (i < length) {
        size_t start;
        if (path[i] == '/') { i += 1; continue; }
        start = i;
        while (i < length && path[i] != '/') i += 1;
        if ((i - start == 1 && path[start] == '.') || (i - start == 2 && path[start] == '.' && path[start + 1] == '.')) return 4;
    }
    return 0;
}
static bool contains(const ghostos_fsd_mount *mount, const uint8_t *path, size_t path_length) {
    size_t i;
    if (mount->path_length == 1 && mount->path[0] == '/') return true;
    if (same_bytes(mount->path, mount->path_length, path, path_length)) return true;
    if (path_length <= mount->path_length || path[mount->path_length] != '/') return false;
    for (i = 0; i < mount->path_length; ++i) if (path[i] != mount->path[i]) return false;
    return true;
}
static int install(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity, const char *text, uint8_t volume, uint64_t generation, bool read_only) {
    size_t length = 0, slot = 0, i;
    bool found = false;
    uint32_t slot_generation;
    while (text[length]) length += 1;
    for (i = 0; i < capacity; ++i) if (!mounts[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    slot_generation = mounts[slot].generation + 1;
    if (!slot_generation) slot_generation = 1;
    mounts[slot].occupied = true;
    mounts[slot].read_only = read_only;
    mounts[slot].host = false;
    mounts[slot].generation = slot_generation;
    mounts[slot].id = namespace->next_mount_id;
    mounts[slot].volume = volume;
    mounts[slot].path_length = (uint8_t)length;
    for (i = 0; i < length; ++i) mounts[slot].path[i] = (uint8_t)text[i];
    namespace->next_mount_id = namespace->next_mount_id == UINT32_MAX ? 1 : namespace->next_mount_id + 1;
    (void)generation;
    return 0;
}
void ghostos_fsd_namespace_init(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity) {
    size_t i;
    namespace->active = false;
    namespace->next_mount_id = 1;
    namespace->authority = 0;
    for (i = 0; i < capacity; ++i) {
        mounts[i].occupied = false;
        mounts[i].read_only = false;
        mounts[i].host = false;
        mounts[i].generation = 0;
        mounts[i].id = 0;
        mounts[i].volume = 0;
        mounts[i].filesystem = 0;
        mounts[i].partition_start = 0;
        mounts[i].partition_length = 0;
        mounts[i].path_length = 0;
    }
}
int ghostos_fsd_namespace_activate(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity, uint64_t generation) {
    int status;
    if (namespace->active) return 1;
    if (capacity < 5) return 3;
    namespace->authority = GHOSTOS_FSD_HOST_AUTHORITY;
    status = install(namespace, mounts, capacity, "/", 0, generation, false);
    if (!status) status = install(namespace, mounts, capacity, "/packages", 1, generation, true);
    if (!status) status = install(namespace, mounts, capacity, "/logs", 2, generation, false);
    if (!status) status = install(namespace, mounts, capacity, "/data", 3, generation, false);
    if (!status) status = install(namespace, mounts, capacity, "/tmp", 4, generation, false);
    if (status) return status;
    namespace->active = true;
    (void)generation;
    return 0;
}
int ghostos_fsd_namespace_resolve(const ghostos_fsd_namespace *namespace, const ghostos_fsd_mount *mounts, size_t capacity,
    const uint8_t *path, size_t path_length, uint8_t *volume, bool *read_only, uint8_t *mount_length) {
    size_t i, best = 0;
    bool found = false;
    int status;
    if (!namespace->active) return 2;
    status = path_ok(path, path_length);
    if (status) return status;
    for (i = 0; i < capacity; ++i) {
        if (!mounts[i].occupied || !contains(&mounts[i], path, path_length)) continue;
        if (!found || mounts[i].path_length >= mounts[best].path_length) { best = i; found = true; }
    }
    if (!found) return 6;
    *volume = mounts[best].volume;
    *read_only = mounts[best].read_only;
    *mount_length = mounts[best].path_length;
    return 0;
}
int ghostos_fsd_namespace_unmount(ghostos_fsd_mount *mounts, size_t capacity, uint64_t capability) {
    uint32_t generation = (uint32_t)(capability >> 32);
    uint32_t raw = (uint32_t)capability;
    size_t index;
    if (!generation || !raw) return 7;
    index = (size_t)raw - 1;
    if (index >= capacity || !mounts[index].occupied || mounts[index].generation != generation) return 7;
    if (index < 5) return 9;
    mounts[index].occupied = false;
    return 0;
}
int ghostos_fsd_namespace_mount_host(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity,
    uint64_t authority, const uint8_t *path, size_t path_length, uint8_t filesystem, uint64_t partition_start,
    uint64_t partition_length, uint64_t *capability) {
    size_t slot = 0, i;
    bool found = false;
    uint32_t generation;
    int status;
    if (!namespace->active) return 2;
    if (authority != namespace->authority) return 8;
    if (!partition_length || partition_start > UINT64_MAX - partition_length) return 5;
    status = path_ok(path, path_length);
    if (status) return status;
    for (i = 0; i < capacity; ++i) if (mounts[i].occupied && same_bytes(mounts[i].path, mounts[i].path_length, path, path_length)) return 11;
    for (i = 0; i < capacity; ++i) if (!mounts[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    generation = mounts[slot].generation + 1;
    if (!generation) generation = 1;
    mounts[slot].occupied = true;
    mounts[slot].read_only = true;
    mounts[slot].host = true;
    mounts[slot].generation = generation;
    mounts[slot].id = namespace->next_mount_id;
    mounts[slot].volume = 0;
    mounts[slot].filesystem = filesystem;
    mounts[slot].partition_start = partition_start;
    mounts[slot].partition_length = partition_length;
    mounts[slot].path_length = (uint8_t)path_length;
    for (i = 0; i < path_length; ++i) mounts[slot].path[i] = path[i];
    namespace->next_mount_id = namespace->next_mount_id == UINT32_MAX ? 1 : namespace->next_mount_id + 1;
    *capability = ((uint64_t)generation << 32) | (slot + 1);
    return 0;
}
int ghostos_fsd_remove_directory(const ghostos_fsd_namespace *namespace, const ghostos_fsd_mount *mounts, size_t capacity,
    const uint8_t *path, size_t path_length, uint16_t rights) {
    uint8_t volume = 0, mount_length = 0;
    bool read_only = false;
    size_t i;
    int status;
    if ((rights & 14u) != 14u) return 8;
    status = path_ok(path, path_length);
    if (status) return status;
    if (path_length == 1 && path[0] == '/') return 8;
    for (i = 0; i < capacity; ++i) {
        if (mounts[i].occupied && same_bytes(mounts[i].path, mounts[i].path_length, path, path_length)) return 8;
    }
    status = ghostos_fsd_namespace_resolve(namespace, mounts, capacity, path, path_length, &volume, &read_only, &mount_length);
    if (status) return status;
    if (read_only) return 10;
    return 0;
}
