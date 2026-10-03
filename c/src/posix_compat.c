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
