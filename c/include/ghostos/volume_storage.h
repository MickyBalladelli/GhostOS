#ifndef GHOSTOS_VOLUME_STORAGE_H
#define GHOSTOS_VOLUME_STORAGE_H
#include "ghostos/volume_mutation.h"
#include "ghostos/volume_generation.h"
/* Serialized synchronous block I/O. Callbacks borrow one 4096-byte block
   during their call and return zero on success. Supply distinct 4096-byte
   map/header scratch buffers, disjoint from the arena and tree scratch.
   Results: 0 success, 1 corrupt/configuration, 2 sequence overflow,
   3 I/O failure, 4 interrupted after durable publication, 5 scratch capacity.
   Failed writes/flushes do not update bank state or sequence output; the
   inactive on-device bank may be partially overwritten. This flushes an
   already-published in-memory mutation, not an atomic mutation transaction.
   Callers must validate full filesystem consistency before flushing. */
typedef struct {
    void *context;
    int (*write_block)(void *context, uint64_t block, const uint8_t *bytes);
    int (*flush)(void *context);
    bool (*interrupted)(void *context);
} ghostos_volume_storage_io;
int ghostos_volume_storage_flush(const ghostos_volume_mutation *mutation,
    ghostos_volume *banks, uint64_t next_checkpoint,
    const ghostos_volume_storage_io *io, uint8_t *map, size_t map_capacity,
    uint8_t *header, size_t header_capacity, uint64_t *sequence);
#endif
