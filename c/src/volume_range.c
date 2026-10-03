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
static int resolve_file(const ghostos_volume_range_file *files, size_t file_count, const uint8_t *path, size_t path_length, size_t *resolved) {
    uint8_t current[GHOSTOS_VOLUME_RANGE_PATH];
    size_t length, depth, i;
    if (path_length > GHOSTOS_VOLUME_RANGE_PATH) return 4;
    for (i = 0; i < path_length; ++i) current[i] = path[i];
    length = path_length;
    for (depth = 0; depth < GHOSTOS_VOLUME_RANGE_DEPTH; ++depth) {
        size_t name_length = 0;
        uint32_t version = 0;
        bool latest = true;
        int status = classify(current, length, &name_length, &version, &latest);
        int index;
        if (status) return status;
        index = find_file(files, file_count, current, name_length, version, latest);
        if (index < 0) return 1;
        if (files[index].file_type == 3) {
            size_t target_length = (size_t)files[index].size;
            if (!files[index].data || target_length > GHOSTOS_VOLUME_RANGE_PATH || !target_length || files[index].data[0] != '/') return 4;
            for (i = 0; i < target_length; ++i) current[i] = files[index].data[i];
            length = target_length;
            continue;
        }
        *resolved = (size_t)index;
        return 0;
    }
    return 9;
}

int ghostos_volume_read_at(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint64_t offset, uint8_t *output, size_t output_capacity, size_t *read) {
    size_t index;
    int status = resolve_file(files, file_count, path, path_length, &index);
    if (status) return status;
    return read_blocks(&files[index], blocks, block_count, offset, output, output_capacity, read);
}
int ghostos_volume_read(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    const ghostos_volume_range_file *file;
    size_t index, copied = 0, steps = 0, size;
    uint32_t id;
    int status = resolve_file(files, file_count, path, path_length, &index);
    if (status) return status;
    file = &files[index];
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
