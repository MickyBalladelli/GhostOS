#include "ghostos/posix_compat.h"
_Static_assert(sizeof(ghostos_posix_fd) == 16, "POSIX fd ABI");
_Static_assert(offsetof(ghostos_posix_fd, occupied) == 10, "POSIX occupancy ABI");
static bool slot(const ghostos_posix_fd *entries, size_t capacity, int32_t fd, size_t *index) {
    if (fd < 3) return false;
    *index = (size_t)(fd - 3);
    return *index < capacity && entries[*index].occupied;
}
bool ghostos_posix_fd_has_free(const ghostos_posix_fd *entries, size_t capacity) {
    for (size_t i = 0; i < capacity; ++i) if (!entries[i].occupied) return true;
    return false;
}
bool ghostos_posix_fd_insert(ghostos_posix_fd *entries, size_t capacity,
    uint8_t access, bool append, int32_t *fd) {
    for (size_t i = 0; i < capacity; ++i) {
        if (entries[i].occupied) continue;
        if (i > (size_t)INT32_MAX - 3) return false;
        *fd = (int32_t)i + 3;
        entries[i] = (ghostos_posix_fd){0, access, append, true};
        return true;
    }
    return false;
}
int ghostos_posix_fd_get(const ghostos_posix_fd *entries, size_t capacity,
    int32_t fd, uint8_t required, ghostos_posix_fd *entry) {
    size_t index;
    if (!slot(entries, capacity, fd, &index)) return 1;
    ghostos_posix_fd value = entries[index];
    if ((required == 1 && value.access == 1) || (required == 2 && value.access == 0)) return 2;
    *entry = value;
    return 0;
}
bool ghostos_posix_fd_offset(ghostos_posix_fd *entries, size_t capacity, int32_t fd, uint64_t offset) {
    size_t index;
    if (!slot(entries, capacity, fd, &index)) return false;
    entries[index].offset = offset;
    return true;
}
bool ghostos_posix_fd_remove(ghostos_posix_fd *entries, size_t capacity, int32_t fd, ghostos_posix_fd *entry) {
    size_t index;
    if (!slot(entries, capacity, fd, &index)) return false;
    *entry = entries[index];
    entries[index] = (ghostos_posix_fd){0};
    return true;
}

static bool mount_matches(const uint8_t *path, size_t length, const char *mount, size_t size) {
    if (length < size) return false;
    for (size_t i = 0; i < size; ++i) if (path[i] != (uint8_t)mount[i]) return false;
    return length == size || path[size] == '/';
}
int ghostos_posix_pseudo_parse(const uint8_t *path, size_t length,
    uint8_t logical[GHOSTOS_POSIX_PSEUDO_NAME_BYTES], uint8_t *logical_length, uint8_t *kind) {
    const char *prefix;
    size_t mount, prefix_size;
    if (mount_matches(path, length, "/proc", 5)) { *kind = 0; mount = 5; prefix = "PROC_"; prefix_size = 5; }
    else if (mount_matches(path, length, "/sys", 4)) { *kind = 1; mount = 4; prefix = "SYS_"; prefix_size = 4; }
    else if (mount_matches(path, length, "/dev", 4)) { *kind = 2; mount = 4; prefix = "DEV_"; prefix_size = 4; }
    else return 1;
    for (size_t i = 0; i < length; ++i) if (!path[i] || path[i] == '\\') return 2;
    const uint8_t *suffix;
    size_t suffix_size;
    if (length == mount) { suffix = (const uint8_t *)"ROOT"; suffix_size = 4; }
    else { suffix = path + mount + 1; suffix_size = length - mount - 1; }
    if (!suffix_size) return 2;
    size_t start = 0;
    for (size_t i = 0; i <= suffix_size; ++i) {
        if (i < suffix_size && suffix[i] != '/') continue;
        size_t part = i - start;
        if (!part || (part == 1 && suffix[start] == '.') ||
            (part == 2 && suffix[start] == '.' && suffix[start + 1] == '.')) return 2;
        start = i + 1;
    }
    for (size_t i = 0; i < GHOSTOS_POSIX_PSEUDO_NAME_BYTES; ++i) logical[i] = 0;
    for (size_t i = 0; i < prefix_size; ++i) logical[i] = (uint8_t)prefix[i];
    size_t used = prefix_size;
    for (size_t i = 0; i < suffix_size; ++i) {
        uint8_t byte = suffix[i];
        if (byte == '/') byte = '_';
        else if (byte >= 'a' && byte <= 'z') byte -= 'a' - 'A';
        else if (!((byte >= 'A' && byte <= 'Z') || (byte >= '0' && byte <= '9') ||
            byte == '_' || byte == '-' || byte == '.')) return 2;
        if (used == GHOSTOS_POSIX_PSEUDO_NAME_BYTES) return 3;
        logical[used++] = byte;
    }
    *logical_length = (uint8_t)used;
    return 0;
}
