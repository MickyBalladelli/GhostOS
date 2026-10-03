#include "ghostos/volume_list.h"
enum { GHOSTOS_VOLUME_LIST_PATH = 192, GHOSTOS_VOLUME_LIST_DEPTH = 40 };
static bool same_name(const ghostos_volume_list_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int latest_live(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t length) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !same_name(&records[i], path, length)) continue;
        if (found < 0 || records[i].version > records[found].version) found = (int)i;
    }
    return found;
}
static int invalid_path(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > GHOSTOS_VOLUME_LIST_PATH || path[length - 1] == '/') return 1;
    if (path[0] == '/') i = 1;
    while (i < length) {
        size_t start = i;
        while (i < length && path[i] != '/') {
            if (path[i] == 0 || path[i] == ';') return 1;
            i += 1;
        }
        if (i == start || (i - start == 1 && path[start] == '.') || (i - start == 2 && path[start] == '.' && path[start + 1] == '.')) return 1;
        if (i < length) i += 1;
    }
    return 0;
}
static int classify(const uint8_t *path, size_t length, size_t *name_length, uint32_t *version, bool *latest) {
    size_t semi = length, i;
    uint32_t parsed = 0;
    while (semi > 0) {
        semi -= 1;
        if (path[semi] == ';') break;
    }
    if (semi < length && path[semi] == ';') {
        if (semi + 1 == length) return 3;
        for (i = semi + 1; i < length; ++i) {
            if (path[i] < '0' || path[i] > '9') return 3;
            if (parsed > (UINT32_MAX - (uint32_t)(path[i] - '0')) / 10) return 3;
            parsed = parsed * 10 + (uint32_t)(path[i] - '0');
        }
        if (invalid_path(path, semi)) return 4;
        *name_length = semi;
        *version = parsed;
        *latest = parsed == 0;
        return 0;
    }
    if (invalid_path(path, length)) return 4;
    *name_length = length;
    *latest = true;
    return 0;
}
static bool child_component(const uint8_t *directory, size_t directory_length, const uint8_t *file, size_t file_length, const uint8_t **component, size_t *component_length) {
    size_t i, start;
    if (directory_length == 1 && directory[0] == '/') {
        if (file_length < 2 || file[0] != '/') return false;
        for (i = 1; i < file_length; ++i) if (file[i] == '/') return false;
        *component = file + 1;
        *component_length = file_length - 1;
        return true;
    }
    if (file_length <= directory_length + 1) return false;
    for (i = 0; i < directory_length; ++i) if (file[i] != directory[i]) return false;
    if (file[directory_length] != '/') return false;
    start = directory_length + 1;
    for (i = start; i < file_length; ++i) if (file[i] == '/') return false;
    *component = file + start;
    *component_length = file_length - start;
    return *component_length != 0;
}
static int compare_component(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i, length = left_length < right_length ? left_length : right_length;
    for (i = 0; i < length; ++i) {
        if (left[i] < right[i]) return -1;
        if (left[i] > right[i]) return 1;
    }
    if (left_length < right_length) return -1;
    if (left_length > right_length) return 1;
    return 0;
}
static int resolve_directory(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t path_length, uint8_t *resolved, size_t *resolved_length) {
    uint8_t current[GHOSTOS_VOLUME_LIST_PATH], next[GHOSTOS_VOLUME_LIST_PATH];
    size_t length, depth, i;
    if (path_length == 1 && path[0] == '/') {
        resolved[0] = '/';
        *resolved_length = 1;
        return 0;
    }
    if (path_length > GHOSTOS_VOLUME_LIST_PATH) return 4;
    for (i = 0; i < path_length; ++i) current[i] = path[i];
    length = path_length;
    for (depth = 0; depth < GHOSTOS_VOLUME_LIST_DEPTH; ++depth) {
        size_t name_length = 0;
        uint32_t version = 0;
        bool latest = true;
        int status = classify(current, length, &name_length, &version, &latest);
        int index = -1;
        size_t record;
        if (status) return status;
        for (record = 0; record < count; ++record) {
            if (!records[record].occupied || records[record].deleted || !same_name(&records[record], current, name_length)) continue;
            if (!latest && records[record].version != version) continue;
            if (index < 0 || records[record].version > records[index].version) index = (int)record;
        }
        if (index < 0) return 1;
        if (records[index].file_type == 3) {
            size_t target_length = (size_t)records[index].size;
            const uint8_t *target = records[index].data;
            if (!target || target_length > GHOSTOS_VOLUME_LIST_PATH) return 4;
            if (target[0] == '/') {
                for (i = 0; i < target_length; ++i) next[i] = target[i];
                length = target_length;
            } else {
                size_t slash = name_length;
                while (slash > 0 && current[slash - 1] != '/') slash -= 1;
                if (slash + target_length > GHOSTOS_VOLUME_LIST_PATH) return 4;
                for (i = 0; i < slash; ++i) next[i] = current[i];
                for (i = 0; i < target_length; ++i) next[slash + i] = target[i];
                length = slash + target_length;
            }
            for (i = 0; i < length; ++i) current[i] = next[i];
            continue;
        }
        if (records[index].file_type != 2) return 2;
        for (i = 0; i < name_length; ++i) resolved[i] = current[i];
        *resolved_length = name_length;
        return 0;
    }
    return 9;
}
int ghostos_volume_list_directory_page(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t path_length, size_t skip, ghostos_volume_list_entry *entries, size_t capacity, size_t *written, size_t *next, bool *has_next) {
    uint8_t directory[GHOSTOS_VOLUME_LIST_PATH], previous[GHOSTOS_VOLUME_LIST_PATH];
    size_t directory_length = 0, previous_length = 0, seen = 0;
    bool have_previous = false;
    int status = resolve_directory(records, count, path, path_length, directory, &directory_length);
    *written = 0;
    *has_next = false;
    *next = 0;
    if (status) return status;
    for (;;) {
        size_t i;
        int best = -1;
        const uint8_t *best_component = 0;
        size_t best_length = 0;
        for (i = 0; i < count; ++i) {
            const uint8_t *component;
            size_t component_length;
            if (!records[i].occupied || records[i].deleted) continue;
            if (latest_live(records, count, records[i].name, records[i].name_length) != (int)i) continue;
            if (!child_component(directory, directory_length, records[i].name, records[i].name_length, &component, &component_length)) continue;
            if (have_previous && compare_component(component, component_length, previous, previous_length) <= 0) continue;
            if (best >= 0 && compare_component(component, component_length, best_component, best_length) >= 0) continue;
            best = (int)i;
            best_component = component;
            best_length = component_length;
        }
        if (best < 0) return 0;
        if (best_length > GHOSTOS_VOLUME_LIST_PATH) return 4;
        for (i = 0; i < best_length; ++i) previous[i] = best_component[i];
        previous_length = best_length;
        have_previous = true;
        if (seen < skip) { seen += 1; continue; }
        if (*written == capacity) {
            *has_next = true;
            *next = skip + *written;
            return 0;
        }
        for (i = 0; i < best_length; ++i) entries[*written].name[i] = best_component[i];
        entries[*written].name_length = (uint8_t)best_length;
        entries[*written].file_type = records[best].file_type;
        entries[*written].mode = records[best].mode;
        entries[*written].version = records[best].version;
        entries[*written].link_count = records[best].link_count ? records[best].link_count : 1;
        entries[*written].size = records[best].size;
        *written += 1;
        seen += 1;
    }
}
int ghostos_volume_list_directory(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t path_length, ghostos_volume_list_entry *entries, size_t capacity, size_t *written) {
    size_t next = 0;
    bool has_next = false;
    int status = ghostos_volume_list_directory_page(records, count, path, path_length, 0, entries, capacity, written, &next, &has_next);
    if (status) return status;
    if (has_next) return 5;
    return 0;
}
