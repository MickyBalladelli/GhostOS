#ifndef GHOSTOS_FSD_OPEN_H
#define GHOSTOS_FSD_OPEN_H
#include "ghostos/fsd_handles.h"
#include "ghostos/fsd_namespace.h"
/* Flags preserve the existing filesystem request wire bits. */
enum {
    GHOSTOS_FSD_OPEN_READ = 1,
    GHOSTOS_FSD_OPEN_WRITE = 2,
    GHOSTOS_FSD_OPEN_CREATE = 4,
    GHOSTOS_FSD_OPEN_TRUNCATE = 8,
    GHOSTOS_FSD_OPEN_APPEND = 16,
    GHOSTOS_FSD_OPEN_DELETE = 32,
    GHOSTOS_FSD_OPEN_ADMIN = 64,
    GHOSTOS_FSD_OPEN_EXCLUSIVE = 512,
    GHOSTOS_FSD_OPEN_ALREADY_EXISTS = 24
};
typedef struct {
    bool occupied, read_only;
    uint8_t name[GHOSTOS_VOLUME_NAME];
    uint16_t name_length;
} ghostos_fsd_named_mount;
/* Mutate the filesystem synchronously and rebuild the supplied reader on
   success. Paths are borrowed only during this call and must not be retained.
   Results use local read/handle/open codes, not volume_write.h's code table.
   Mutation failures may retain backend side effects, matching filesystem write.
   The backend owns block allocation, records, quotas, and persistence. */
typedef int (*ghostos_fsd_write_empty_fn)(void *context, ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length);
typedef struct {
    ghostos_fsd_handles *handles;
    ghostos_volume_reader *reader;
    const ghostos_fsd_namespace *namespace;
    const ghostos_fsd_mount *mounts;
    size_t mount_count;
    const ghostos_fsd_named_mount *named_mounts;
    size_t named_mount_count;
    ghostos_fsd_write_empty_fn write_empty;
    void *context;
} ghostos_fsd_open_state;
typedef struct {
    uint64_t capability;
    uint16_t rights;
    ghostos_volume_record metadata;
} ghostos_fsd_open_info;
/* Caller serializes tables, callbacks, and storage. Open returns a copied
   metadata result only on success. Creation/truncation can remain committed
   if a later mode check or final handle allocation fails. */
int ghostos_fsd_open(ghostos_fsd_open_state *state, uint64_t process,
    uint64_t authority, const uint8_t *path, size_t path_length,
    uint16_t flags, ghostos_fsd_open_info *info);
int ghostos_fsd_create_file(ghostos_fsd_open_state *state, uint64_t process,
    uint64_t authority, const uint8_t *path, size_t path_length, ghostos_fsd_open_info *info);
ghostos_status ghostos_fsd_open_status(int result);
#endif
