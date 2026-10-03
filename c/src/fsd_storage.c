#include "ghostos/fsd_storage.h"
int ghostos_fsd_storage_write_empty(void *context, ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length) {
    int status = ghostos_volume_mutation_write_empty(context, reader, path, path_length);
    switch (status) {
    case GHOSTOS_VOLUME_MUTATION_QUOTA:
    case GHOSTOS_VOLUME_MUTATION_ARENA_FULL: return GHOSTOS_FSD_STORAGE_NO_SPACE;
    case GHOSTOS_VOLUME_MUTATION_OVERFLOW: return GHOSTOS_FSD_HANDLES_VERSION_OVERFLOW;
    case GHOSTOS_VOLUME_MUTATION_SCRATCH: return GHOSTOS_FSD_HANDLES_FILE_FULL;
    case GHOSTOS_VOLUME_MUTATION_INVALID: return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    default: return status;
    }
}
ghostos_status ghostos_fsd_storage_status(int result) {
    if (result == GHOSTOS_FSD_STORAGE_NO_SPACE) return GHOSTOS_STATUS_NO_SPACE;
    return ghostos_fsd_open_status(result);
}
ghostos_fsd_checkpoint_backend ghostos_fsd_storage_checkpoints(
    ghostos_volume_mutation *volume, uint64_t *next_id) {
    ghostos_fsd_checkpoint_backend backend = {
        .pins = volume->pins, .capacity = volume->pin_capacity,
        .next_id = next_id, .generation = &volume->generation, .root = &volume->root
    };
    return backend;
}
