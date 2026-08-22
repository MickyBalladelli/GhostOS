#ifndef GHOSTOS_POSIX_H
#define GHOSTOS_POSIX_H

#include <stddef.h>
#include <stdint.h>

/*
 * Optional C ABI for a GhostOS process runtime. The loader supplies one table per
 * process. Calls remain scoped to that process and never enter a global POSIX
 * namespace.
 */
struct ghostos_posix_api {
    void *context;
    int32_t (*open)(void *context, const uint8_t *path, size_t length, uint32_t flags);
    int32_t (*close)(void *context, int32_t fd);
    intptr_t (*read)(void *context, int32_t fd, uint8_t *output, size_t length);
    intptr_t (*write)(void *context, int32_t fd, const uint8_t *input, size_t length);
    int64_t (*seek)(void *context, int32_t fd, int64_t offset, uint32_t origin);
    uint64_t (*monotonic_time_ns)(void *context);
};

enum ghostos_open_flags {
    GHOSTOS_O_RDONLY = 0,
    GHOSTOS_O_WRONLY = 1u << 0,
    GHOSTOS_O_RDWR = 1u << 1,
    GHOSTOS_O_CREAT = 1u << 2,
    GHOSTOS_O_TRUNC = 1u << 3,
    GHOSTOS_O_APPEND = 1u << 4
};

enum ghostos_linux_architecture {
    GHOSTOS_LINUX_X86_64 = 0,
    GHOSTOS_LINUX_AARCH64 = 1
};

struct ghostos_linux_syscall_request {
    uint32_t architecture;
    uint64_t number;
    uint64_t arguments[6];
    uint64_t process_id;
};

struct ghostos_linux_syscall_response {
    int64_t value;
    uint8_t exited;
};

#endif
