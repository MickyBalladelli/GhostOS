#ifndef GHOSTOS_FSD_LOCK_H
#define GHOSTOS_FSD_LOCK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 lock busy, 2 lock table full, 3 access denied, 4 invalid path.
   Mode: shared=0, exclusive=1. A whole-file range overlaps every record. */
#define GHOSTOS_FSD_PATH 64u
typedef struct {
    bool occupied, whole;
    uint8_t mode;
    uint32_t generation;
    uint64_t owner, record;
    uint8_t path[GHOSTOS_FSD_PATH];
    uint8_t path_length;
} ghostos_fsd_lock_slot;
int ghostos_fsd_lock(ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, const uint8_t *path, size_t path_length,
    bool whole, uint64_t record, uint8_t mode, uint64_t *handle);
int ghostos_fsd_unlock(ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, uint64_t handle);
int ghostos_fsd_io(const ghostos_fsd_lock_slot *locks, size_t capacity, uint64_t owner, const uint8_t *path, size_t path_length,
    uint64_t record, uint8_t mode);
#endif
