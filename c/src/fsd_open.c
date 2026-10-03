#include "ghostos/fsd_open.h"
static bool valid_utf8(const uint8_t *bytes, size_t length) {
    size_t i = 0;
    while (i < length) {
        uint8_t first = bytes[i++];
        uint32_t value, minimum;
        size_t remaining;
        if (first < 0x80) continue;
        if (first >= 0xc2 && first <= 0xdf) { value = first & 0x1f; minimum = 0x80; remaining = 1; }
        else if (first >= 0xe0 && first <= 0xef) { value = first & 0x0f; minimum = 0x800; remaining = 2; }
        else if (first >= 0xf0 && first <= 0xf4) { value = first & 0x07; minimum = 0x10000; remaining = 3; }
        else return false;
        if (remaining > length - i) return false;
        while (remaining--) {
            uint8_t next = bytes[i++];
            if ((next & 0xc0) != 0x80) return false;
            value = (value << 6) | (next & 0x3f);
        }
        if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return false;
    }
    return true;
}
static int name_ok(const uint8_t *path, size_t length) {
    size_t i;
    if (!length || length > GHOSTOS_VOLUME_NAME || path[length - 1] == '/') return 4;
    for (i = 0; i < length; ++i) {
        if (!path[i] || (i + 1 < length && path[i] == '/' && path[i + 1] == '/')) return 4;
    }
    return valid_utf8(path, length) ? 0 : 4;
}
static bool has_version(const uint8_t *path, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) if (path[i] == ';') return true;
    return false;
}
static uint16_t requested_rights(uint16_t flags) {
    uint16_t rights = flags & (GHOSTOS_FSD_OPEN_READ | GHOSTOS_FSD_OPEN_WRITE);
    if (flags & GHOSTOS_FSD_OPEN_DELETE) rights |= GHOSTOS_FSD_RIGHT_DELETE;
    if (flags & GHOSTOS_FSD_OPEN_ADMIN) rights |= GHOSTOS_FSD_RIGHT_ADMIN;
    return rights;
}
static int read_only(const ghostos_fsd_open_state *state,
    const uint8_t *path, size_t length, bool strip_version, bool *result) {
    size_t i, mount_length = length;
    bool readonly = false;
    if (path[0] == '/') {
        uint8_t volume, prefix;
        int status;
        if (strip_version) {
            for (i = length; i > 0; --i) if (path[i - 1] == ';') { mount_length = i - 1; break; }
        }
        if (!state->namespace) return 1;
        status = ghostos_fsd_namespace_resolve(state->namespace, state->mounts,
            state->mount_count, path, mount_length, &volume, &readonly, &prefix);
        if (status == 4) return 4;
        if (status) return 1;
    }
    for (i = 0; i < state->named_mount_count; ++i) {
        const ghostos_fsd_named_mount *mount = &state->named_mounts[i];
        size_t byte;
        bool same = true;
        if (!mount->occupied || !mount->read_only || mount->name_length > length ||
            mount->name_length > GHOSTOS_VOLUME_NAME) continue;
        for (byte = 0; byte < mount->name_length; ++byte)
            if (path[byte] != mount->name[byte]) { same = false; break; }
        if (same && (mount->name_length == length || path[mount->name_length] == ':' ||
            path[mount->name_length] == '/')) readonly = true;
    }
    *result = readonly;
    return 0;
}
static int parent_access(const ghostos_fsd_open_state *state, uint64_t process,
    const uint8_t *path, size_t length) {
    size_t end, parent = 0, i;
    uint16_t required = GHOSTOS_FSD_RIGHT_WRITE | GHOSTOS_FSD_RIGHT_TRAVERSE;
    for (i = 0; i < length; ++i) if (path[i] == '/') parent = i;
    if (!parent) return ghostos_fsd_mode_access(state->handles->processes,
        state->handles->process_count, process, 0777, required);
    for (end = 1; end <= parent; ++end) {
        const ghostos_volume_record *metadata;
        int status;
        if (end != parent && path[end] != '/') continue;
        status = ghostos_volume_reader_lookup_following(state->reader, path, end, &metadata);
        if (status) return status;
        if (metadata->file_type != 2) return 2;
        status = ghostos_fsd_mode_access(state->handles->processes, state->handles->process_count,
            process, metadata->mode, end == parent ? required : GHOSTOS_FSD_RIGHT_TRAVERSE);
        if (status) return status;
    }
    return 0;
}
static int write_empty(ghostos_fsd_open_state *state, const uint8_t *path, size_t length) {
    if (!state->write_empty) return GHOSTOS_FSD_HANDLES_INVALID_ARGUMENT;
    return state->write_empty(state->context, state->reader, path, length);
}
static int install(ghostos_fsd_open_state *state, uint64_t process,
    const uint8_t *path, size_t length, const ghostos_volume_record *metadata,
    uint16_t rights, bool readonly, bool append, ghostos_fsd_open_info *info) {
    ghostos_volume_record handle_metadata = *metadata;
    uint64_t capability;
    size_t i;
    int status = name_ok(path, length);
    if (status) return status;
    handle_metadata.name_length = (uint16_t)length;
    for (i = 0; i < length; ++i) handle_metadata.name[i] = path[i];
    status = ghostos_fsd_install_file(state->handles, process, &handle_metadata,
        rights, readonly, append, &capability);
    if (status) return status;
    info->capability = capability;
    info->rights = rights;
    info->metadata = *metadata;
    if (!info->metadata.file_type) info->metadata.file_type = 1;
    if (!info->metadata.link_count) info->metadata.link_count = 1;
    return 0;
}
static int create_named(ghostos_fsd_open_state *state, uint64_t process, uint64_t authority,
    const uint8_t *path, size_t length, uint16_t rights, bool readonly, bool exclusive,
    ghostos_fsd_open_info *info) {
    const ghostos_volume_record *metadata;
    size_t i;
    int status = ghostos_fsd_authorize_process(state->handles, process, authority, GHOSTOS_FSD_RIGHT_WRITE);
    if (status) return status;
    if (readonly) return GHOSTOS_FSD_HANDLES_READ_ONLY;
    for (i = 0; i < state->handles->file_count; ++i) if (!state->handles->files[i].occupied) break;
    if (i == state->handles->file_count) return GHOSTOS_FSD_HANDLES_FILE_FULL;
    if (exclusive && !ghostos_volume_reader_lookup(state->reader, path, length, &metadata))
        return GHOSTOS_FSD_OPEN_ALREADY_EXISTS;
    status = parent_access(state, process, path, length);
    if (status) return status;
    status = write_empty(state, path, length);
    if (status) return status;
    status = ghostos_volume_reader_lookup(state->reader, path, length, &metadata);
    if (status) return status;
    status = ghostos_fsd_mode_access(state->handles->processes, state->handles->process_count,
        process, metadata->mode, rights);
    if (status) return status;
    return install(state, process, path, length, metadata, rights, false, false, info);
}
int ghostos_fsd_open(ghostos_fsd_open_state *state, uint64_t process,
    uint64_t authority, const uint8_t *path, size_t length, uint16_t flags,
    ghostos_fsd_open_info *info) {
    uint16_t rights = requested_rights(flags);
    const ghostos_volume_record *metadata;
    ghostos_volume_record resolved;
    bool readonly, exists;
    int status;
    if (!rights) return GHOSTOS_FSD_READ_ACCESS_DENIED;
    status = ghostos_fsd_authorize_process(state->handles, process, authority, rights);
    if (status) return status;
    status = name_ok(path, length);
    if (status) return status;
    if ((rights & GHOSTOS_FSD_RIGHT_WRITE) && has_version(path, length)) return 4;
    status = read_only(state, path, length, true, &readonly);
    if (status) return status;
    if (readonly && (rights & GHOSTOS_FSD_RIGHT_WRITE)) return GHOSTOS_FSD_HANDLES_READ_ONLY;
    if ((flags & (GHOSTOS_FSD_OPEN_CREATE | GHOSTOS_FSD_OPEN_EXCLUSIVE)) ==
        (GHOSTOS_FSD_OPEN_CREATE | GHOSTOS_FSD_OPEN_EXCLUSIVE))
        return create_named(state, process, authority, path, length, rights, readonly, true, info);
    exists = !ghostos_volume_reader_lookup(state->reader, path, length, &metadata);
    if (!exists && !(flags & GHOSTOS_FSD_OPEN_CREATE)) return 1;
    if (!exists) {
        status = ghostos_fsd_authorize_process(state->handles, process, authority, GHOSTOS_FSD_RIGHT_WRITE);
        if (status) return status;
        status = parent_access(state, process, path, length);
        if (status) return status;
        status = write_empty(state, path, length);
        if (status) return status;
        status = ghostos_volume_reader_lookup_following(state->reader, path, length, &metadata);
        if (status) return status;
        status = ghostos_fsd_mode_access(state->handles->processes, state->handles->process_count,
            process, metadata->mode, rights);
        if (status) return status;
        return install(state, process, metadata->name, metadata->name_length,
            metadata, rights, readonly, (flags & GHOSTOS_FSD_OPEN_APPEND) != 0, info);
    }
    status = ghostos_volume_reader_lookup_following(state->reader, path, length, &metadata);
    if (status) return status;
    status = ghostos_fsd_mode_access(state->handles->processes, state->handles->process_count,
        process, metadata->mode, rights);
    if (status) return status;
    status = name_ok(metadata->name, metadata->name_length);
    if (status) return status;
    resolved = *metadata;
    if (flags & GHOSTOS_FSD_OPEN_TRUNCATE) {
        status = ghostos_fsd_authorize_process(state->handles, process, authority, GHOSTOS_FSD_RIGHT_WRITE);
        if (status) return status;
        status = ghostos_fsd_check_io_lock(state->handles->locks, state->handles->lock_count,
            process, resolved.name, resolved.name_length, true, 0, 1);
        if (status) return status;
        status = write_empty(state, resolved.name, resolved.name_length);
        if (status) return status;
    }
    status = ghostos_volume_reader_lookup_following(state->reader, path, length, &metadata);
    if (status) return status;
    if ((flags & GHOSTOS_FSD_OPEN_CREATE) && metadata->file_type > 1) return 2;
    status = ghostos_fsd_mode_access(state->handles->processes, state->handles->process_count,
        process, metadata->mode, rights);
    if (status) return status;
    return install(state, process, resolved.name, resolved.name_length,
        metadata, rights, readonly, (flags & GHOSTOS_FSD_OPEN_APPEND) != 0, info);
}
int ghostos_fsd_create_file(ghostos_fsd_open_state *state, uint64_t process,
    uint64_t authority, const uint8_t *path, size_t length, ghostos_fsd_open_info *info) {
    bool readonly;
    int status;
    if (has_version(path, length)) return 4;
    status = name_ok(path, length);
    if (status) return status;
    status = read_only(state, path, length, false, &readonly);
    if (status) return status;
    return create_named(state, process, authority, path, length,
        GHOSTOS_FSD_RIGHT_READ | GHOSTOS_FSD_RIGHT_WRITE, readonly, false, info);
}
ghostos_status ghostos_fsd_open_status(int result) {
    if (result == GHOSTOS_FSD_OPEN_ALREADY_EXISTS) return GHOSTOS_STATUS_ALREADY_EXISTS;
    return ghostos_fsd_handles_status(result);
}
