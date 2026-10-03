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
   Reachable live/checkpoint consistency is checked before flushing. Supply
   data_owners consistency scratch in the mutation backend. */
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
/* Load into fresh caller-owned storage, never buffers backing a live reader.
   Reads both complete banks into staging before selection. Highest valid
   sequence wins; equal valid sequences are corrupt. A corrupt candidate falls
   back to the older bank. Scalar outputs publish only on success; staging,
   arena/cache/scratch arrays may change on failure. No reservation is made;
   the caller owns device reservation and staging lifetime.
   staging requires 2 * (block_count + 2) * 4096 bytes and must be disjoint
   from every destination buffer. Results use the flush code table. */
int ghostos_volume_storage_load(ghostos_volume_mutation *mutation,
    ghostos_volume_reader *reader, ghostos_volume *banks, uint64_t *next_checkpoint,
    void *context, int (*read_block)(void *context, uint64_t block, uint8_t *bytes),
    uint8_t *staging, size_t staging_capacity);
#endif
