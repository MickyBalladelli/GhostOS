#ifndef GHOSTOS_FSD_CHECKPOINT_H
#define GHOSTOS_FSD_CHECKPOINT_H
#include "ghostos/fsd_handles.h"
#include "ghostos/volume_snapshot.h"
/* Checkpoint callbacks for the C volume pin table. Install these callbacks on
   ghostos_fsd_handles and pass this backend as its context. Backend and storage
   remain caller-owned and serialized. generation supplies the current volume
   generation at creation; next_id supplies the next checkpoint identifier. */
typedef struct {
    ghostos_volume_pin_slot *pins;
    size_t capacity;
    uint64_t *next_id;
    const uint64_t *generation;
} ghostos_fsd_checkpoint_backend;
int ghostos_fsd_checkpoint_create(void *context, uint64_t *checkpoint, uint64_t *generation);
int ghostos_fsd_checkpoint_release(void *context, uint64_t checkpoint);
#endif
