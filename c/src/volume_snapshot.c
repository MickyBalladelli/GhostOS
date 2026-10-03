#include "ghostos/volume_snapshot.h"
static bool same_name(const ghostos_volume_file *file, const uint8_t *name, size_t name_length) {
    size_t i;
    if (file->name_length != name_length) return false;
    for (i = 0; i < name_length; ++i) if (file->name[i] != name[i]) return false;
    return true;
}
static int copy_selected(const ghostos_volume_file *file, uint8_t *output, size_t output_capacity, size_t *read) {
    size_t i;
    if (file->data_length > output_capacity) return 4;
    for (i = 0; i < file->data_length; ++i) output[i] = file->data[i];
    *read = file->data_length;
    return 0;
}
static int select_file(const ghostos_volume_file *files, size_t count, const uint8_t *name, size_t name_length, uint64_t generation, const ghostos_volume_file **selected) {
    size_t i;
    bool found = false;
    for (i = 0; i < count; ++i) {
        if (files[i].deleted || files[i].generation > generation || !same_name(&files[i], name, name_length)) continue;
        if (!found || files[i].version > (*selected)->version) {
            *selected = &files[i];
            found = true;
        }
    }
    return found ? 0 : 3;
}
int ghostos_volume_pin(ghostos_volume_pin *pins, size_t capacity, uint64_t *next_id, uint64_t generation, uint64_t *id) {
    size_t i;
    if (!*next_id) return 2;
    if (*next_id == UINT64_MAX) return 2;
    for (i = 0; i < capacity; ++i) if (!pins[i].occupied) {
        pins[i].occupied = true;
        pins[i].id = *next_id;
        pins[i].generation = generation;
        *id = *next_id;
        *next_id += 1;
        return 0;
    }
    return 1;
}
int ghostos_volume_unpin(ghostos_volume_pin *pins, size_t capacity, uint64_t id) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (pins[i].occupied && pins[i].id == id) { pins[i].occupied = false; return 0; }
    return 3;
}
int ghostos_volume_read(const ghostos_volume_file *files, size_t count, const uint8_t *name, size_t name_length, uint8_t *output, size_t output_capacity, size_t *read) {
    const ghostos_volume_file *selected = 0;
    int status = select_file(files, count, name, name_length, UINT64_MAX, &selected);
    if (status) return status;
    return copy_selected(selected, output, output_capacity, read);
}
int ghostos_volume_snapshot_read(const ghostos_volume_pin *pins, size_t pin_count, const ghostos_volume_file *files, size_t file_count, uint64_t id, const uint8_t *name, size_t name_length, uint8_t *output, size_t output_capacity, size_t *read) {
    const ghostos_volume_file *selected = 0;
    size_t i;
    bool found = false;
    for (i = 0; i < pin_count; ++i) if (pins[i].occupied && pins[i].id == id) { found = true; break; }
    if (!found) return 3;
    if (select_file(files, file_count, name, name_length, pins[i].generation, &selected)) return 3;
    return copy_selected(selected, output, output_capacity, read);
}
