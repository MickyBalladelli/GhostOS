#ifndef SYNOS_POSIX_H
#define SYNOS_POSIX_H

#include <stddef.h>
#include <stdint.h>

/*
 * Optional C ABI for a SynOS process runtime. The loader supplies one table per
 * process. Calls remain scoped to that process and never enter a global POSIX
 * namespace.
 */
struct synos_posix_api {
    void *context;
    int32_t (*open)(void *context, const uint8_t *path, size_t length, uint32_t flags);
    int32_t (*close)(void *context, int32_t fd);
    intptr_t (*read)(void *context, int32_t fd, uint8_t *output, size_t length);
    intptr_t (*write)(void *context, int32_t fd, const uint8_t *input, size_t length);
    int64_t (*seek)(void *context, int32_t fd, int64_t offset, uint32_t origin);
    uint64_t (*monotonic_time_ns)(void *context);
};

enum synos_open_flags {
    SYNOS_O_RDONLY = 0,
    SYNOS_O_WRONLY = 1u << 0,
    SYNOS_O_RDWR = 1u << 1,
    SYNOS_O_CREAT = 1u << 2,
    SYNOS_O_TRUNC = 1u << 3,
    SYNOS_O_APPEND = 1u << 4
};

#endif
