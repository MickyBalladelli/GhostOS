#include "ghostos/fsd_handles.h"
static uint32_t next_generation(uint32_t generation) {
    generation += 1;
    return generation ? generation : 1;
}
static uint64_t token(size_t slot, uint32_t generation) {
    return ((uint64_t)generation << 32) | ((uint64_t)slot + 1);
}
static bool same_path(const uint8_t *left, size_t left_length,
    const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
int ghostos_fsd_check_io_lock(const ghostos_fsd_read_lock *locks, size_t count,
    uint64_t process, const uint8_t *path, size_t path_length,
    bool whole, uint64_t record, uint8_t mode) {
    size_t i;
    for (i = 0; i < count; ++i) {
        const ghostos_fsd_read_lock *lock = &locks[i];
        if (lock->occupied && lock->owner != process &&
            same_path(lock->path, lock->path_length, path, path_length) &&
            (lock->whole || whole || lock->record == record) &&
            (lock->mode == 1 || mode == 1)) return GHOSTOS_FSD_READ_LOCK_BUSY;
    }
    return 0;
}
int ghostos_fsd_lock_file(ghostos_fsd_handles *handles, uint64_t process,
    uint64_t file, bool whole, uint64_t record, uint8_t mode, uint64_t *capability) {
    size_t index, i, byte;
    const ghostos_fsd_read_file *open;
    uint16_t required;
    int status;
    if (mode > 1) return GHOSTOS_FSD_HANDLES_INVALID_LOCK;
    required = mode == 1 ? GHOSTOS_FSD_RIGHT_WRITE : GHOSTOS_FSD_RIGHT_READ;
    status = ghostos_fsd_file_index(handles->files, handles->file_count,
        process, file, required, &index);
    if (status) return status;
    open = &handles->files[index];
    if (open->path_length > GHOSTOS_VOLUME_NAME) return 4;
    status = ghostos_fsd_check_io_lock(handles->locks, handles->lock_count,
        process, open->path, open->path_length, whole, record, mode);
    if (status) return status;
    for (i = 0; i < handles->lock_count; ++i) {
        ghostos_fsd_read_lock *slot = &handles->locks[i];
        if (slot->occupied) continue;
        slot->generation = next_generation(slot->generation);
        slot->occupied = true;
        slot->owner = process;
        slot->file = file;
        slot->path_length = open->path_length;
        for (byte = 0; byte < open->path_length; ++byte) slot->path[byte] = open->path[byte];
        slot->whole = whole;
        slot->record = record;
        slot->mode = mode;
        *capability = token(i, slot->generation);
        return 0;
    }
    return GHOSTOS_FSD_HANDLES_LOCK_FULL;
}
int ghostos_fsd_unlock_file(ghostos_fsd_handles *handles,
    uint64_t process, uint64_t capability) {
    uint32_t raw = (uint32_t)capability;
    size_t index;
    ghostos_fsd_read_lock *slot;
    if (!raw) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    index = (size_t)raw - 1;
    if (index >= handles->lock_count) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    slot = &handles->locks[index];
    if (!slot->occupied || slot->generation != (uint32_t)(capability >> 32) || slot->owner != process)
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    slot->occupied = false;
    return 0;
}
int ghostos_fsd_map_file(ghostos_fsd_handles *handles, const ghostos_volume_reader *reader,
    uint64_t process, uint64_t file, uint64_t offset, uint64_t length,
    bool writable, uint64_t *capability) {
    size_t index, i, byte;
    const ghostos_fsd_read_file *open;
    const ghostos_volume_record *metadata;
    uint16_t required = GHOSTOS_FSD_RIGHT_READ | (writable ? GHOSTOS_FSD_RIGHT_WRITE : 0);
    int status;
    if (offset % 4096u || !length || length % 4096u || length > ((uint64_t)UINT32_MAX << 16))
        return GHOSTOS_FSD_HANDLES_INVALID_REQUEST;
    status = ghostos_fsd_file_index(handles->files, handles->file_count,
        process, file, required, &index);
    if (status) return status;
    open = &handles->files[index];
    if (writable && open->read_only_mount) return GHOSTOS_FSD_HANDLES_READ_ONLY;
    if (open->path_length > GHOSTOS_VOLUME_NAME) return 4;
    status = ghostos_volume_reader_lookup_following(reader, open->path, open->path_length, &metadata);
    if (status) return status;
    if ((metadata->file_type && metadata->file_type != 1) || offset > metadata->size || offset > UINT64_MAX - length)
        return GHOSTOS_FSD_HANDLES_INVALID_REQUEST;
    status = ghostos_fsd_mode_access(handles->processes, handles->process_count,
        process, metadata->mode, required);
    if (status) return status;
    status = ghostos_fsd_check_io_lock(handles->locks, handles->lock_count,
        process, open->path, open->path_length, true, 0, writable ? 1 : 0);
    if (status) return status;
    for (i = 0; i < handles->mapping_count; ++i) {
        ghostos_fsd_mapping_slot *slot = &handles->mappings[i];
        if (slot->occupied) continue;
        slot->generation = next_generation(slot->generation);
        slot->occupied = true;
        slot->owner = process;
        slot->file = file;
        slot->path_length = open->path_length;
        for (byte = 0; byte < open->path_length; ++byte) slot->path[byte] = open->path[byte];
        slot->offset = offset;
        slot->length = length;
        slot->writable = writable;
        *capability = token(i, slot->generation) | 0x80000000ull;
        return 0;
    }
    return GHOSTOS_FSD_HANDLES_FILE_FULL;
}
int ghostos_fsd_unmap_file(ghostos_fsd_handles *handles,
    uint64_t process, uint64_t capability) {
    uint32_t raw = (uint32_t)capability;
    size_t index;
    ghostos_fsd_mapping_slot *slot;
    if (!(raw & 0x80000000u) || !(raw & 0x7fffffffu)) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    index = (size_t)(raw & 0x7fffffffu) - 1;
    if (index >= handles->mapping_count) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    slot = &handles->mappings[index];
    if (!slot->occupied || slot->generation != (uint32_t)(capability >> 32) || slot->owner != process)
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    slot->occupied = false;
    return 0;
}
int ghostos_fsd_create_snapshot(ghostos_fsd_handles *handles, uint64_t process,
    uint64_t authority, uint64_t *capability, uint64_t *generation) {
    size_t i;
    int status = ghostos_fsd_authorize_process(handles, process, authority, GHOSTOS_FSD_RIGHT_ADMIN);
    if (status) return status;
    for (i = 0; i < handles->snapshot_count; ++i) {
        ghostos_fsd_checkpoint_slot *slot = &handles->snapshots[i];
        uint64_t checkpoint, checkpoint_generation;
        if (slot->occupied) continue;
        if (!handles->create_checkpoint) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
        status = handles->create_checkpoint(handles->context, &checkpoint, &checkpoint_generation);
        if (status) return status;
        slot->generation = next_generation(slot->generation);
        slot->occupied = true;
        slot->owner = process;
        slot->checkpoint = checkpoint;
        slot->checkpoint_generation = checkpoint_generation;
        *capability = token(i, slot->generation);
        *generation = checkpoint_generation;
        return 0;
    }
    return GHOSTOS_FSD_HANDLES_SNAPSHOT_FULL;
}
int ghostos_fsd_release_snapshot(ghostos_fsd_handles *handles,
    uint64_t process, uint64_t capability) {
    uint32_t raw = (uint32_t)capability;
    size_t index;
    ghostos_fsd_checkpoint_slot *slot;
    int status;
    if (!raw) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    index = (size_t)raw - 1;
    if (index >= handles->snapshot_count) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    slot = &handles->snapshots[index];
    if (!slot->occupied || slot->generation != (uint32_t)(capability >> 32) || slot->owner != process)
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    status = handles->release_checkpoint(handles->context, slot->checkpoint);
    if (status) return status;
    slot->occupied = false;
    return 0;
}
