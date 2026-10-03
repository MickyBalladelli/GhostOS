#include "ghostos/fsd_handles.h"
int ghostos_fsd_read(const ghostos_fsd_read_state *state,
    uint64_t process, uint64_t capability, uint64_t offset,
    uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    size_t index;
    const ghostos_fsd_read_file *file;
    const ghostos_volume_record *metadata;
    int status;
    if (output_capacity > GHOSTOS_FSD_READ_MAX_BUFFER) {
        *required = GHOSTOS_FSD_READ_MAX_BUFFER;
        return GHOSTOS_FSD_READ_BUFFER_TOO_LARGE;
    }
    status = ghostos_fsd_file_index(state->files, state->file_count,
        process, capability, GHOSTOS_FSD_RIGHT_READ, &index);
    if (status) return status;
    file = &state->files[index];
    if (file->path_length > GHOSTOS_VOLUME_NAME) return 4;
    status = ghostos_volume_reader_lookup_following(state->reader,
        file->path, file->path_length, &metadata);
    if (status) return status;
    status = ghostos_fsd_mode_access(state->processes, state->process_count,
        process, metadata->mode, GHOSTOS_FSD_RIGHT_READ);
    if (status) return status;
    status = ghostos_fsd_check_io_lock(state->locks, state->lock_count,
        process, file->path, file->path_length, false, offset, 0);
    if (status) return status;
    return ghostos_volume_reader_read_at(state->reader, file->path,
        file->path_length, offset, output, output_capacity, read);
}
ghostos_status ghostos_fsd_read_status(int result) {
    switch (result) {
    case 0: return GHOSTOS_STATUS_NORMAL;
    case 1: return GHOSTOS_STATUS_NOT_FOUND;
    case 2: return GHOSTOS_STATUS_NOT_DIRECTORY;
    case 3:
    case 4:
    case 9: return GHOSTOS_STATUS_INVALID_PATH;
    case 5: return GHOSTOS_STATUS_CORRUPT;
    case 6:
    case GHOSTOS_FSD_READ_BUFFER_TOO_LARGE:
        return GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 1u, 0u);
    case GHOSTOS_FSD_READ_ACCESS_DENIED:
    case GHOSTOS_FSD_READ_INVALID_CAPABILITY:
    case GHOSTOS_FSD_READ_PROCESS_NOT_REGISTERED: return GHOSTOS_STATUS_ACCESS_DENIED;
    case GHOSTOS_FSD_READ_LOCK_BUSY: return GHOSTOS_STATUS_BUSY;
    default: return GHOSTOS_STATUS_INVALID_ARGUMENT;
    }
}
