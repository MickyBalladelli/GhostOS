#ifndef GHOSTOS_FSD_SNAPSHOT_H
#define GHOSTOS_FSD_SNAPSHOT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 access denied, 2 invalid capability, 3 invalid path,
   4 snapshot table full, 5 admin required, 6 checkpoint release failed.
   Create requires the admin right, bit 3. The path region is 192 bytes.
   Release clears the slot only after the checkpoint release succeeds. */
#define GHOSTOS_FSD_SNAPSHOT_PATH 192u
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t owner;
    const uint8_t *const *names;
    const uint8_t *name_lengths;
    size_t name_count;
} ghostos_fsd_snapshot;
int ghostos_fsd_snapshot_create(uint16_t rights, uint64_t owner, ghostos_fsd_snapshot *snapshots, size_t capacity,
    const uint8_t *const *names, const uint8_t *name_lengths, size_t name_count, uint64_t *handle);
int ghostos_fsd_snapshot_list(const ghostos_fsd_snapshot *snapshots, size_t capacity, uint64_t owner, uint64_t handle,
    const uint8_t *buffer, size_t buffer_length, uint64_t path_length, size_t skip, uint8_t *output, size_t output_capacity,
    size_t *written, size_t *next);
int ghostos_fsd_snapshot_release(ghostos_fsd_snapshot *snapshots, size_t capacity, uint64_t owner, uint64_t handle,
    bool checkpoint_ready);
#endif
