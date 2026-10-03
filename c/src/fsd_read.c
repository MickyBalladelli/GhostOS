#include "ghostos/fsd_read.h"
static bool same_path(const uint8_t *left, size_t left_length,
    const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int file_index(const ghostos_fsd_read_state *state, uint64_t process,
    uint64_t capability, size_t *index) {
    uint32_t raw = (uint32_t)capability;
    const ghostos_fsd_read_file *file;
    if (!raw) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    *index = (size_t)raw - 1;
    if (*index >= state->file_count) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    file = &state->files[*index];
    if (!file->occupied || file->generation != (uint32_t)(capability >> 32) ||
        file->owner != process || !(file->rights & 1u)) return GHOSTOS_FSD_READ_ACCESS_DENIED;
    return 0;
}
static int mode_access(const ghostos_fsd_read_state *state, uint64_t process,
    uint16_t mode) {
    size_t i;
    uint16_t bits;
    for (i = 0; i < state->process_count; ++i) {
        if (!state->processes[i].occupied || state->processes[i].process != process) continue;
        if (state->processes[i].rights & 8u) return 0;
        bits = process == 1 ? (uint16_t)(mode >> 6) : mode;
        return bits & 4u ? 0 : GHOSTOS_FSD_READ_ACCESS_DENIED;
    }
    return GHOSTOS_FSD_READ_PROCESS_NOT_REGISTERED;
}
int ghostos_fsd_read(const ghostos_fsd_read_state *state,
    uint64_t process, uint64_t capability, uint64_t offset,
    uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    size_t index, i;
    const ghostos_fsd_read_file *file;
    const ghostos_volume_record *metadata;
    int status;
    if (output_capacity > GHOSTOS_FSD_READ_MAX_BUFFER) {
        *required = GHOSTOS_FSD_READ_MAX_BUFFER;
        return GHOSTOS_FSD_READ_BUFFER_TOO_LARGE;
    }
    status = file_index(state, process, capability, &index);
    if (status) return status;
    file = &state->files[index];
    if (file->path_length > GHOSTOS_VOLUME_NAME) return 4;
    status = ghostos_volume_reader_lookup_following(state->reader,
        file->path, file->path_length, &metadata);
    if (status) return status;
    status = mode_access(state, process, metadata->mode);
    if (status) return status;
    for (i = 0; i < state->lock_count; ++i) {
        const ghostos_fsd_read_lock *lock = &state->locks[i];
        if (lock->occupied && lock->owner != process && lock->mode == 1 &&
            (lock->whole || lock->record == offset) &&
            same_path(lock->path, lock->path_length, file->path, file->path_length)) return GHOSTOS_FSD_READ_LOCK_BUSY;
    }
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
