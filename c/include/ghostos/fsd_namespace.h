#ifndef GHOSTOS_FSD_NAMESPACE_H
#define GHOSTOS_FSD_NAMESPACE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 already active, 2 inactive, 3 capacity, 4 invalid path,
   5 invalid partition, 6 not found, 7 invalid capability, 8 access denied,
   9 root busy, 10 read only, 11 already mounted.
   Volume: root=0, packages=1, logs=2, data=3, temporary=4.
   Host filesystem: ext4=0, fat32=1, ntfs=2.
   Removal requires delete|write|admin, bits 2|4|8. */
#define GHOSTOS_FSD_NAMESPACE_PATH 96u
#define GHOSTOS_FSD_HOST_AUTHORITY 0x4e53504143450001ull
typedef struct {
    bool active;
    uint32_t next_mount_id;
    uint64_t authority;
} ghostos_fsd_namespace;
typedef struct {
    bool occupied, read_only, host;
    uint32_t generation, id;
    uint8_t volume, filesystem, path_length;
    uint64_t partition_start, partition_length;
    uint8_t path[GHOSTOS_FSD_NAMESPACE_PATH];
} ghostos_fsd_mount;
void ghostos_fsd_namespace_init(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity);
int ghostos_fsd_namespace_activate(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity, uint64_t generation);
int ghostos_fsd_namespace_resolve(const ghostos_fsd_namespace *namespace, const ghostos_fsd_mount *mounts, size_t capacity,
    const uint8_t *path, size_t path_length, uint8_t *volume, bool *read_only, uint8_t *mount_length);
int ghostos_fsd_namespace_unmount(ghostos_fsd_mount *mounts, size_t capacity, uint64_t capability);
int ghostos_fsd_namespace_mount_host(ghostos_fsd_namespace *namespace, ghostos_fsd_mount *mounts, size_t capacity,
    uint64_t authority, const uint8_t *path, size_t path_length, uint8_t filesystem, uint64_t partition_start,
    uint64_t partition_length, uint64_t *capability);
int ghostos_fsd_remove_directory(const ghostos_fsd_namespace *namespace, const ghostos_fsd_mount *mounts, size_t capacity,
    const uint8_t *path, size_t path_length, uint16_t rights);
#endif
