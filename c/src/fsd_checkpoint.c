#include "ghostos/fsd_checkpoint.h"
int ghostos_fsd_checkpoint_create(void *context, uint64_t *checkpoint, uint64_t *generation) {
    ghostos_fsd_checkpoint_backend *backend = context;
    uint64_t id, current;
    int status;
    if (!backend || !backend->next_id || !backend->generation ||
        (backend->capacity && !backend->pins)) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    current = *backend->generation;
    status = ghostos_volume_pin(backend->pins, backend->capacity, backend->next_id, current, &id);
    if (status == 1) return GHOSTOS_FSD_HANDLES_SNAPSHOT_FULL;
    if (status == 2) return GHOSTOS_FSD_HANDLES_VERSION_OVERFLOW;
    if (status) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    *checkpoint = id;
    *generation = current;
    return 0;
}
int ghostos_fsd_checkpoint_release(void *context, uint64_t checkpoint) {
    ghostos_fsd_checkpoint_backend *backend = context;
    int status;
    if (!backend || (backend->capacity && !backend->pins)) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    status = ghostos_volume_unpin(backend->pins, backend->capacity, checkpoint);
    return status == 3 ? 1 : status;
}
