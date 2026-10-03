#ifndef GHOSTOS_FSD_HANDLES_H
#define GHOSTOS_FSD_HANDLES_H
#include "ghostos/fsd_read.h"
/* Caller-owned daemon tables, serialized by the caller. Local read results
   retain their values; 15 process table full, 16 file table full,
   17 invalid table configuration. Capacities may be zero.
   Init is for fresh daemon startup only; it does not release existing slots. */
enum {
    GHOSTOS_FSD_HANDLES_PROCESS_FULL = 15,
    GHOSTOS_FSD_HANDLES_FILE_FULL = 16,
    GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT = 17
};
enum {
    GHOSTOS_FSD_RIGHT_READ = 1,
    GHOSTOS_FSD_RIGHT_WRITE = 2,
    GHOSTOS_FSD_RIGHT_DELETE = 4,
    GHOSTOS_FSD_RIGHT_ADMIN = 8,
    GHOSTOS_FSD_RIGHT_TRAVERSE = 16
};
typedef struct {
    bool occupied, writable;
    uint32_t generation;
    uint64_t owner, file, offset, length;
    uint8_t path[GHOSTOS_VOLUME_NAME];
    uint16_t path_length;
} ghostos_fsd_mapping_slot;
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t owner, checkpoint, checkpoint_generation;
} ghostos_fsd_checkpoint_slot;
typedef int (*ghostos_fsd_checkpoint_release_fn)(void *context, uint64_t checkpoint);
typedef struct {
    ghostos_fsd_read_process *processes;
    size_t process_count;
    ghostos_fsd_read_file *files;
    size_t file_count;
    ghostos_fsd_mapping_slot *mappings;
    size_t mapping_count;
    ghostos_fsd_read_lock *locks;
    size_t lock_count;
    ghostos_fsd_checkpoint_slot *snapshots;
    size_t snapshot_count;
    ghostos_fsd_checkpoint_release_fn release_checkpoint;
    void *context;
} ghostos_fsd_handles;
int ghostos_fsd_handles_init(ghostos_fsd_handles *handles);
/* Re-registering replaces rights without changing the authority token.
   Reused slots increment generation, wrapping zero to one. Process IDs are
   nonzero. Output tokens change only on success. */
int ghostos_fsd_register_process(ghostos_fsd_handles *handles, uint64_t process,
    uint16_t rights, uint64_t *capability);
int ghostos_fsd_authorize_process(const ghostos_fsd_handles *handles,
    uint64_t process, uint64_t authority, uint16_t required);
/* Internal allocation step after caller authorization, namespace, lookup,
   creation/truncation, and mode checks. Copies the resolved metadata name.
   It grants no additional rights and performs no filesystem mutation. */
int ghostos_fsd_install_file(ghostos_fsd_handles *handles, uint64_t process,
    const ghostos_volume_record *metadata, uint16_t rights,
    bool read_only_mount, bool append, uint64_t *capability);
int ghostos_fsd_file_index(const ghostos_fsd_read_file *files, size_t count,
    uint64_t process, uint64_t capability, uint16_t required, size_t *index);
int ghostos_fsd_mode_access(const ghostos_fsd_read_process *processes, size_t count,
    uint64_t process, uint16_t mode, uint16_t required);
/* Close invalidates only mappings/locks tied to this exact file token.
   Unregister invalidates all owner files, mappings, locks, and snapshots.
   Snapshot callbacks run before clearing each snapshot, and release errors
   are ignored during process teardown, preserving daemon cleanup behavior.
   A release callback is required if the snapshot table has nonzero capacity.
   Callbacks must not reenter or mutate these tables. */
int ghostos_fsd_close_file(ghostos_fsd_handles *handles,
    uint64_t process, uint64_t capability);
int ghostos_fsd_unregister_process(ghostos_fsd_handles *handles, uint64_t process);
ghostos_fsd_read_state ghostos_fsd_handles_read_state(const ghostos_fsd_handles *handles,
    const ghostos_volume_reader *reader);
ghostos_status ghostos_fsd_handles_status(int result);
#endif
