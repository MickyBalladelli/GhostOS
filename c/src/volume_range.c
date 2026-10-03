#include "ghostos/volume_range.h"
enum { GHOSTOS_VOLUME_RANGE_PATH = 192, GHOSTOS_VOLUME_RANGE_DATA = 4080, GHOSTOS_VOLUME_RANGE_DEPTH = 40 };
static bool same_name(const ghostos_volume_range_file *file, const uint8_t *path, size_t length) {
    size_t i;
    if (file->name_length != length) return false;
    for (i = 0; i < length; ++i) if (file->name[i] != path[i]) return false;
    return true;
}
static int invalid_path(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > GHOSTOS_VOLUME_RANGE_PATH || path[length - 1] == '/') return 1;
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
static int find_file(const ghostos_volume_range_file *files, size_t count, const uint8_t *path, size_t length, uint32_t version, bool latest) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (!files[i].occupied || files[i].deleted || !same_name(&files[i], path, length)) continue;
        if (!latest && files[i].version != version) continue;
        if (found < 0 || files[i].version > files[found].version) found = (int)i;
    }
    return found;
}
static uint64_t checksum(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static int read_blocks(const ghostos_volume_range_file *file, const ghostos_volume_range_block *blocks, size_t block_count, uint64_t offset, uint8_t *output, size_t output_capacity, size_t *read) {
    uint64_t available, block_start = 0;
    size_t wanted, copied = 0, steps = 0;
    uint32_t id;
    if (file->file_type == 2) return 2;
    if (offset > file->size) return 3;
    available = file->size - offset;
    wanted = available < output_capacity ? (size_t)available : output_capacity;
    id = file->first_block;
    while (id && copied < wanted) {
        const ghostos_volume_range_block *block;
        uint64_t block_end;
        size_t length, start, amount, i;
        if (id > block_count || steps == block_count) return 5;
        block = &blocks[id - 1];
        steps += 1;
        length = block->length;
        if (!length || length > GHOSTOS_VOLUME_RANGE_DATA || !block->bytes || checksum(block->bytes, length) != block->checksum) return 5;
        block_end = block_start > UINT64_MAX - length ? UINT64_MAX : block_start + length;
        if (offset < block_end) {
            uint64_t relative = offset > block_start ? offset - block_start : 0;
            if (relative > length) return 5;
            start = (size_t)relative;
            amount = length - start;
            if (amount > wanted - copied) amount = wanted - copied;
            for (i = 0; i < amount; ++i) output[copied + i] = block->bytes[start + i];
            copied += amount;
        }
        block_start = block_end;
        id = block->next;
    }
    if (copied != wanted) return 5;
    *read = copied;
    return 0;
}
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
        while (remaining) {
            uint8_t next = bytes[i++];
            if ((next & 0xc0) != 0x80) return false;
            value = (value << 6) | (next & 0x3f);
            remaining -= 1;
        }
        if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return false;
    }
    return true;
}
static int lookup_file(const ghostos_volume_range_file *files, size_t count, const uint8_t *path, size_t length, size_t *resolved) {
    size_t name_length = 0;
    uint32_t version = 0;
    bool latest = true;
    int index, status = classify(path, length, &name_length, &version, &latest);
    if (status) return status;
    index = find_file(files, count, path, name_length, version, latest);
    if (index < 0) return 1;
    *resolved = (size_t)index;
    return 0;
}
static int append_components(const uint8_t *source, size_t source_length, uint8_t *output, size_t *length) {
    size_t i = 0;
    while (i < source_length) {
        size_t start = i, component, j;
        while (i < source_length && source[i] != '/') i += 1;
        component = i - start;
        if (!component || (component == 1 && source[start] == '.')) {
        } else if (component == 2 && source[start] == '.' && source[start + 1] == '.') {
            if (*length > 1) {
                size_t end = *length - 1;
                while (end > 0) {
                    end -= 1;
                    if (output[end] == '/') break;
                }
                *length = end + 1;
            }
        } else {
            size_t separator = *length > 1 ? 1 : 0;
            if (component + separator > GHOSTOS_VOLUME_RANGE_PATH - *length || !valid_utf8(source + start, component)) return 4;
            if (separator) output[(*length)++] = '/';
            for (j = 0; j < component; ++j) output[*length + j] = source[start + j];
            *length += component;
        }
        if (i < source_length) i += 1;
    }
    return 0;
}
static int resolve_file(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, size_t *resolved) {
    uint8_t current[GHOSTOS_VOLUME_RANGE_PATH], next[GHOSTOS_VOLUME_RANGE_PATH], target[GHOSTOS_VOLUME_RANGE_PATH];
    size_t length, depth, i;
    if (!path_length || path_length > GHOSTOS_VOLUME_RANGE_PATH) return 4;
    for (i = 0; i < path_length; ++i) current[i] = path[i];
    length = path_length;
    for (depth = 0; depth < GHOSTOS_VOLUME_RANGE_DEPTH; ++depth) {
        size_t component_end = 1, link_end = 0, index = 0;
        int status;
        if (!valid_utf8(current, length)) return 4;
        while (component_end <= length) {
            if ((component_end == length || current[component_end] == '/') && component_end > 1) {
                status = lookup_file(files, file_count, current, component_end, &index);
                if (!status && files[index].file_type == 3) { link_end = component_end; break; }
                if (status && status != 1) return status;
            }
            component_end += 1;
        }
        if (!link_end) return lookup_file(files, file_count, current, length, resolved);
        {
            const ghostos_volume_range_file *link = &files[index];
            size_t target_length, parent_end = 0, joined = 1, scan = link_end, read = 0;
            if (link->size > GHOSTOS_VOLUME_RANGE_PATH) return 5;
            target_length = (size_t)link->size;
            if (link->first_block || !link->data) {
                status = read_blocks(link, blocks, block_count, 0, target, target_length, &read);
                if (status) return status;
                if (read != target_length) return 5;
            } else {
                for (i = 0; i < target_length; ++i) target[i] = link->data[i];
            }
            if (!valid_utf8(target, target_length)) return 5;
            while (scan > 0) {
                scan -= 1;
                if (current[scan] == '/') { parent_end = scan; break; }
            }
            next[0] = '/';
            if (!target_length || target[0] != '/') {
                status = append_components(current, parent_end ? parent_end : 1, next, &joined);
                if (status) return status;
            }
            status = append_components(target, target_length, next, &joined);
            if (status) return status;
            status = append_components(current + link_end, length - link_end, next, &joined);
            if (status) return status;
            for (i = 0; i < joined; ++i) current[i] = next[i];
            length = joined;
        }
    }
    return 9;
}

int ghostos_volume_read_at(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint64_t offset, uint8_t *output, size_t output_capacity, size_t *read) {
    size_t index;
    int status = resolve_file(files, file_count, blocks, block_count, path, path_length, &index);
    if (status) return status;
    return read_blocks(&files[index], blocks, block_count, offset, output, output_capacity, read);
}
static int read_file(const ghostos_volume_range_file *file, const ghostos_volume_range_block *blocks, size_t block_count, uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    size_t copied = 0, steps = 0, size;
    uint32_t id;
    if (file->file_type == 2) return 2;
    if (file->size > SIZE_MAX) return 5;
    size = (size_t)file->size;
    *required = size;
    if (output_capacity < size) return 6;
    id = file->first_block;
    while (id) {
        const ghostos_volume_range_block *block;
        size_t length, i;
        if (id > block_count || steps == block_count) return 5;
        steps += 1;
        block = &blocks[id - 1];
        length = block->length;
        if (!length || length > GHOSTOS_VOLUME_RANGE_DATA || length > size - copied || !block->bytes || checksum(block->bytes, length) != block->checksum) return 5;
        for (i = 0; i < length; ++i) output[copied + i] = block->bytes[i];
        copied += length;
        id = block->next;
    }
    if (copied != size || checksum(output, copied) != file->checksum) return 5;
    *read = copied;
    return 0;
}

int ghostos_volume_read(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    size_t index;
    int status = resolve_file(files, file_count, blocks, block_count, path, path_length, &index);
    if (status) return status;
    return read_file(&files[index], blocks, block_count, output, output_capacity, read, required);
}
int ghostos_volume_read_version_blocks(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint32_t version, uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    int index;
    if (!version) return 3;
    if (invalid_path(path, path_length) || !valid_utf8(path, path_length)) return 4;
    index = find_file(files, file_count, path, path_length, version, false);
    if (index < 0) return 1;
    return read_file(&files[index], blocks, block_count, output, output_capacity, read, required);
}
