#include "ghostos/fsd_handles.h"
static uint64_t token(size_t index, uint32_t generation) {
    return ((uint64_t)generation << 32) | ((uint64_t)index + 1);
}
static uint32_t next_generation(uint32_t generation) {
    generation += 1;
    return generation ? generation : 1;
}
static int process_index(const ghostos_fsd_read_process *processes, size_t count,
    uint64_t process, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!processes[i].occupied || processes[i].process != process) continue;
        *index = i;
        return 0;
    }
    return GHOSTOS_FSD_READ_PROCESS_NOT_REGISTERED;
}
int ghostos_fsd_handles_init(ghostos_fsd_handles *handles) {
    size_t i;
    if (!handles || handles->process_count > UINT32_MAX ||
        handles->file_count >= 0x80000000ull || handles->lock_count > UINT32_MAX ||
        handles->snapshot_count > UINT32_MAX || handles->mapping_count >= 0x80000000ull ||
        (handles->process_count && !handles->processes) ||
        (handles->file_count && !handles->files) ||
        (handles->mapping_count && !handles->mappings) ||
        (handles->lock_count && !handles->locks) ||
        (handles->snapshot_count && (!handles->snapshots || !handles->release_checkpoint)))
        return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    for (i = 0; i < handles->process_count; ++i)
        handles->processes[i] = (ghostos_fsd_read_process){0};
    for (i = 0; i < handles->file_count; ++i) {
        handles->files[i] = (ghostos_fsd_read_file){0};
        handles->files[i].owner = 1;
    }
    for (i = 0; i < handles->mapping_count; ++i) {
        handles->mappings[i] = (ghostos_fsd_mapping_slot){0};
        handles->mappings[i].owner = 1;
        handles->mappings[i].file = 1;
    }
    for (i = 0; i < handles->lock_count; ++i) {
        handles->locks[i] = (ghostos_fsd_read_lock){0};
        handles->locks[i].owner = 1;
        handles->locks[i].file = 1;
        handles->locks[i].whole = true;
    }
    for (i = 0; i < handles->snapshot_count; ++i) {
        handles->snapshots[i] = (ghostos_fsd_checkpoint_slot){0};
        handles->snapshots[i].owner = 1;
        handles->snapshots[i].checkpoint = 1;
    }
    return 0;
}
int ghostos_fsd_register_process(ghostos_fsd_handles *handles, uint64_t process,
    uint16_t rights, uint64_t *capability) {
    size_t i;
    if (!process) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    for (i = 0; i < handles->process_count; ++i) {
        ghostos_fsd_read_process *slot = &handles->processes[i];
        if (!slot->occupied || slot->process != process) continue;
        slot->rights = rights;
        *capability = token(i, slot->generation);
        return 0;
    }
    for (i = 0; i < handles->process_count; ++i) {
        ghostos_fsd_read_process *slot = &handles->processes[i];
        if (slot->occupied) continue;
        slot->generation = next_generation(slot->generation);
        slot->occupied = true;
        slot->process = process;
        slot->rights = rights;
        *capability = token(i, slot->generation);
        return 0;
    }
    return GHOSTOS_FSD_HANDLES_PROCESS_FULL;
}
int ghostos_fsd_authorize_process(const ghostos_fsd_handles *handles,
    uint64_t process, uint64_t authority, uint16_t required) {
    size_t index;
    int status = process_index(handles->processes, handles->process_count, process, &index);
    if (status) return status;
    if (authority != token(index, handles->processes[index].generation))
        return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    if ((handles->processes[index].rights & required) != required)
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    return 0;
}
int ghostos_fsd_install_file(ghostos_fsd_handles *handles, uint64_t process,
    const ghostos_volume_record *metadata, uint16_t rights,
    bool read_only_mount, bool append, uint64_t *capability) {
    size_t i, byte;
    if (!process) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    if (!metadata->name_length || metadata->name_length > GHOSTOS_VOLUME_NAME) return 4;
    for (byte = 0; byte < metadata->name_length; ++byte) if (!metadata->name[byte]) return 4;
    for (i = 0; i < handles->file_count; ++i) {
        ghostos_fsd_read_file *slot = &handles->files[i];
        if (slot->occupied) continue;
        slot->generation = next_generation(slot->generation);
        slot->occupied = true;
        slot->owner = process;
        slot->rights = rights;
        slot->read_only_mount = read_only_mount;
        slot->append = append;
        slot->path_length = metadata->name_length;
        for (byte = 0; byte < metadata->name_length; ++byte) slot->path[byte] = metadata->name[byte];
        *capability = token(i, slot->generation);
        return 0;
    }
    return GHOSTOS_FSD_HANDLES_FILE_FULL;
}
int ghostos_fsd_file_index(const ghostos_fsd_read_file *files, size_t count,
    uint64_t process, uint64_t capability, uint16_t required, size_t *index) {
    uint32_t raw = (uint32_t)capability;
    size_t slot;
    if (!raw) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    slot = (size_t)raw - 1;
    if (slot >= count) return GHOSTOS_FSD_READ_INVALID_CAPABILITY;
    if (!files[slot].occupied || files[slot].generation != (uint32_t)(capability >> 32) ||
        files[slot].owner != process || (files[slot].rights & required) != required)
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    *index = slot;
    return 0;
}
int ghostos_fsd_mode_access(const ghostos_fsd_read_process *processes, size_t count,
    uint64_t process, uint16_t mode, uint16_t required) {
    size_t index;
    uint16_t bits;
    int status = process_index(processes, count, process, &index);
    if (status) return status;
    if (processes[index].rights & GHOSTOS_FSD_RIGHT_ADMIN) return 0;
    bits = process == 1 ? (uint16_t)(mode >> 6) : mode;
    if ((required & GHOSTOS_FSD_RIGHT_READ) && !(bits & 4u)) return GHOSTOS_FSD_READ_ACCESS_DENIED;
    if ((required & (GHOSTOS_FSD_RIGHT_WRITE | GHOSTOS_FSD_RIGHT_DELETE)) && !(bits & 2u))
        return GHOSTOS_FSD_READ_ACCESS_DENIED;
    if ((required & GHOSTOS_FSD_RIGHT_TRAVERSE) && !(bits & 1u)) return GHOSTOS_FSD_READ_ACCESS_DENIED;
    return 0;
}
int ghostos_fsd_close_file(ghostos_fsd_handles *handles,
    uint64_t process, uint64_t capability) {
    size_t index, i;
    int status = ghostos_fsd_file_index(handles->files, handles->file_count,
        process, capability, 0, &index);
    if (status) return status;
    handles->files[index].occupied = false;
    for (i = 0; i < handles->mapping_count; ++i)
        if (handles->mappings[i].occupied && handles->mappings[i].file == capability)
            handles->mappings[i].occupied = false;
    for (i = 0; i < handles->lock_count; ++i)
        if (handles->locks[i].occupied && handles->locks[i].file == capability)
            handles->locks[i].occupied = false;
    return 0;
}
int ghostos_fsd_unregister_process(ghostos_fsd_handles *handles, uint64_t process) {
    size_t index, i;
    int status = process_index(handles->processes, handles->process_count, process, &index);
    if (status) return status;
    for (i = 0; i < handles->file_count; ++i)
        if (handles->files[i].occupied && handles->files[i].owner == process)
            handles->files[i].occupied = false;
    for (i = 0; i < handles->mapping_count; ++i)
        if (handles->mappings[i].occupied && handles->mappings[i].owner == process)
            handles->mappings[i].occupied = false;
    for (i = 0; i < handles->lock_count; ++i)
        if (handles->locks[i].occupied && handles->locks[i].owner == process)
            handles->locks[i].occupied = false;
    for (i = 0; i < handles->snapshot_count; ++i) {
        ghostos_fsd_checkpoint_slot *slot = &handles->snapshots[i];
        if (!slot->occupied || slot->owner != process) continue;
        (void)handles->release_checkpoint(handles->context, slot->checkpoint);
        slot->occupied = false;
    }
    handles->processes[index].occupied = false;
    handles->processes[index].process = 0;
    return 0;
}
ghostos_fsd_read_state ghostos_fsd_handles_read_state(const ghostos_fsd_handles *handles,
    const ghostos_volume_reader *reader) {
    ghostos_fsd_read_state state = {
        .reader = reader,
        .processes = handles->processes, .process_count = handles->process_count,
        .files = handles->files, .file_count = handles->file_count,
        .locks = handles->locks, .lock_count = handles->lock_count
    };
    return state;
}
ghostos_status ghostos_fsd_handles_status(int result) {
    if (result == GHOSTOS_FSD_HANDLES_PROCESS_FULL || result == GHOSTOS_FSD_HANDLES_FILE_FULL ||
        result == GHOSTOS_FSD_HANDLES_LOCK_FULL || result == GHOSTOS_FSD_HANDLES_SNAPSHOT_FULL ||
        result == GHOSTOS_FSD_HANDLES_STORAGE_FULL)
        return GHOSTOS_STATUS_NO_SPACE;
    if (result == GHOSTOS_FSD_HANDLES_READ_ONLY) return GHOSTOS_STATUS_READ_ONLY;
    if (result == GHOSTOS_FSD_HANDLES_VERSION_OVERFLOW)
        return GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_FATAL, GHOSTOS_FACILITY_FILESYSTEM, 3u, 0u);
    return ghostos_fsd_read_status(result);
}
