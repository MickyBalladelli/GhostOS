#ifndef GHOSTOS_FSD_READ_H
#define GHOSTOS_FSD_READ_H
#include "ghostos/volume_reader.h"
#include "ghostos/status.h"
/* Daemon read boundary over caller-owned, serialized tables.
   Result 0 success; 1..9 use volume_range.h; 10 access denied,
   11 invalid capability, 12 process not registered, 13 lock busy,
   14 IPC buffer too large. No public syscall status values are changed.
   File tokens store generation in the high word and slot + 1 in the low word.
   Read is right bit 0; process admin is bit 3. Process 1 uses owner mode bits,
   all other processes use other mode bits. File READ cannot be bypassed by admin.
   All tables, paths, and reader storage remain borrowed for each operation. */
#define GHOSTOS_FSD_READ_MAX_BUFFER 65536u
enum {
    GHOSTOS_FSD_READ_OK = 0,
    GHOSTOS_FSD_READ_ACCESS_DENIED = 10,
    GHOSTOS_FSD_READ_INVALID_CAPABILITY = 11,
    GHOSTOS_FSD_READ_PROCESS_NOT_REGISTERED = 12,
    GHOSTOS_FSD_READ_LOCK_BUSY = 13,
    GHOSTOS_FSD_READ_BUFFER_TOO_LARGE = 14
};
typedef struct {
    bool occupied;
    uint64_t process;
    uint16_t rights;
} ghostos_fsd_read_process;
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t owner;
    uint16_t rights;
    uint8_t path[GHOSTOS_VOLUME_NAME];
    uint16_t path_length;
} ghostos_fsd_read_file;
typedef struct {
    bool occupied, whole;
    uint8_t mode;
    uint64_t owner, record;
    const uint8_t *path;
    size_t path_length;
} ghostos_fsd_read_lock;
typedef struct {
    const ghostos_volume_reader *reader;
    const ghostos_fsd_read_process *processes;
    size_t process_count;
    const ghostos_fsd_read_file *files;
    size_t file_count;
    const ghostos_fsd_read_lock *locks;
    size_t lock_count;
} ghostos_fsd_read_state;
/* Buffer-size validation precedes handle validation, lookup, mode permission,
   and shared-record lock checks. Locks compare the stored open-file path and
   exact starting offset, preserving daemon semantics even for symlink handles.
   read changes only on success. Failed checksum reads may alter output bytes.
   required receives 65536 only for an oversized IPC buffer. */
int ghostos_fsd_read(const ghostos_fsd_read_state *state,
    uint64_t process, uint64_t capability, uint64_t offset,
    uint8_t *output, size_t output_capacity, size_t *read, size_t *required);
/* Convert local results to the existing filesystem protocol status ABI. */
ghostos_status ghostos_fsd_read_status(int result);
#endif
