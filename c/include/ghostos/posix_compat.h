#ifndef GHOSTOS_POSIX_COMPAT_H
#define GHOSTOS_POSIX_COMPAT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct { uint64_t offset; uint8_t access; bool append, occupied; } ghostos_posix_fd;
/* Access modes: 0 read-only, 1 write-only, 2 read/write.
 * Required access: 0 none, 1 read, 2 write. Result: 0 success, 1 bad fd, 2 denied. */
bool ghostos_posix_fd_has_free(const ghostos_posix_fd *entries, size_t capacity);
bool ghostos_posix_fd_insert(ghostos_posix_fd *entries, size_t capacity,
    uint8_t access, bool append, int32_t *fd);
int ghostos_posix_fd_get(const ghostos_posix_fd *entries, size_t capacity,
    int32_t fd, uint8_t required, ghostos_posix_fd *entry);
bool ghostos_posix_fd_offset(ghostos_posix_fd *entries, size_t capacity, int32_t fd, uint64_t offset);
bool ghostos_posix_fd_remove(ghostos_posix_fd *entries, size_t capacity, int32_t fd, ghostos_posix_fd *entry);
#define GHOSTOS_POSIX_PSEUDO_NAME_BYTES 64u
/* 0 success, 1 not pseudo, 2 invalid path, 3 too long. Kind: proc=0, sys=1, dev=2. */
int ghostos_posix_pseudo_parse(const uint8_t *path, size_t length,
    uint8_t logical[GHOSTOS_POSIX_PSEUDO_NAME_BYTES], uint8_t *logical_length, uint8_t *kind);
#endif
