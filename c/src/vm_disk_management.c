#include "ghostos/vm_disk_management.h"

#include <stdlib.h>
#include <string.h>

static bool same_id(const uint8_t *left, size_t left_length,
    const uint8_t *right, size_t right_length) {
    return left_length == right_length && (!left_length || !memcmp(left, right, left_length));
}

static bool writable_source(const ghostos_vm_disk_spec *spec) {
    return !spec->read_only && spec->persistence == 0;
}

uint32_t ghostos_vm_disk_validate_specs(const ghostos_vm_disk_spec *specs,
    size_t count, const ghostos_vm_disk_paths *paths, size_t *index) {
    bool has_system = false;
    for (size_t i = 0; i < count; ++i) {
        *index = i;
        const ghostos_vm_disk_spec *spec = &specs[i];
        if (spec->empty_id) return 1;
        for (size_t j = 0; j < i; ++j) {
            if (same_id(spec->id, spec->id_length, specs[j].id, specs[j].id_length)) return 2;
        }
        if (spec->role == 0) {
            if (has_system) return 3;
            has_system = true;
        }
        if (spec->bus || spec->slot) return 4;
        for (size_t j = 0; j < i; ++j) {
            if (spec->controller == specs[j].controller && spec->bus == specs[j].bus &&
                spec->slot == specs[j].slot) return 5;
        }
        if (writable_source(spec)) {
            if (!paths->resolve(paths->context, i)) return 6;
            for (size_t j = 0; j < i; ++j) {
                if (writable_source(&specs[j]) && paths->same_path(paths->context, j, i)) return 7;
            }
        }
    }
    return 0;
}

uint32_t ghostos_vm_disk_validate_image(uint32_t format, uint64_t capacity,
    bool has_format, uint32_t expected_format, bool has_capacity, uint64_t expected_capacity) {
    if (has_format && format != expected_format) return 1;
    if (!capacity || capacity % 512) return 2;
    if (has_capacity && capacity != expected_capacity) return 3;
    return 0;
}

bool ghostos_vm_disk_needs_clone(bool read_only, uint32_t persistence) {
    return !read_only && persistence != 0;
}

const char *ghostos_vm_disk_controller_name(uint32_t controller) {
    static const char *const names[] = {"AHCI", "NVMe", "virtio-blk"};
    return controller < 3 ? names[controller] : NULL;
}

static size_t decimal(uint8_t value, char *output) {
    size_t length = 0;
    if (value >= 100) output[length++] = (char)('0' + value / 100);
    if (value >= 10) output[length++] = (char)('0' + (value / 10) % 10);
    output[length++] = (char)('0' + value % 10);
    return length;
}

size_t ghostos_vm_disk_guest_id(uint32_t controller, uint8_t bus, uint8_t slot,
    char *output, size_t capacity) {
    if (controller > 2) return 0;
    const char *prefix = controller == 0 ? "ahci-bus" : controller == 1 ? "nvme-bus" : "virtio-blk-bus";
    const char *suffix = controller == 0 ? "-port" : controller == 1 ? "-namespace1" : "-slot";
    char buffer[32];
    size_t length = strlen(prefix);
    memcpy(buffer, prefix, length);
    length += decimal(bus, buffer + length);
    size_t suffix_length = strlen(suffix);
    memcpy(buffer + length, suffix, suffix_length);
    length += suffix_length;
    if (controller != 1) length += decimal(slot, buffer + length);
    if (capacity <= length) return 0;
    memcpy(output, buffer, length);
    output[length] = 0;
    return length;
}

uint32_t ghostos_vm_disk_clone_temporary(const ghostos_vm_disk_clone_io *io) {
    for (uint32_t attempt = 0; attempt < 32; ++attempt) {
        uint32_t result = io->reserve(io->context, attempt);
        if (result == 1) continue;
        if (result != 0) return 1;
        if (!io->copy(io->context)) {
            io->discard(io->context);
            return 1;
        }
        return 0;
    }
    return 2;
}

typedef struct {
    const uint8_t *id;
    size_t length;
    void *payload;
} disk_entry;

struct ghostos_vm_disk_manager {
    disk_entry *entries;
    size_t length, capacity;
};

ghostos_vm_disk_manager *ghostos_vm_disk_manager_new(void) {
    return calloc(1, sizeof(ghostos_vm_disk_manager));
}

void ghostos_vm_disk_manager_free(ghostos_vm_disk_manager *manager,
    void (*destroy)(void *payload)) {
    if (!manager) return;
    for (size_t i = 0; i < manager->length; ++i) destroy(manager->entries[i].payload);
    free(manager->entries);
    free(manager);
}

size_t ghostos_vm_disk_manager_length(const ghostos_vm_disk_manager *manager) {
    return manager->length;
}

void *ghostos_vm_disk_manager_at(const ghostos_vm_disk_manager *manager, size_t index) {
    return index < manager->length ? manager->entries[index].payload : NULL;
}

static size_t find(const ghostos_vm_disk_manager *manager, const uint8_t *id, size_t length) {
    for (size_t i = 0; i < manager->length; ++i) {
        if (same_id(id, length, manager->entries[i].id, manager->entries[i].length)) return i;
    }
    return manager->length;
}

void *ghostos_vm_disk_manager_get(const ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length) {
    return ghostos_vm_disk_manager_at(manager, find(manager, id, length));
}

static int compare(const disk_entry *left, const disk_entry *right) {
    size_t length = left->length < right->length ? left->length : right->length;
    int order = length ? memcmp(left->id, right->id, length) : 0;
    if (order) return order;
    return left->length < right->length ? -1 : left->length > right->length ? 1 : 0;
}

bool ghostos_vm_disk_manager_insert(ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length, void *payload) {
    if (manager->length == manager->capacity) {
        size_t maximum = SIZE_MAX / sizeof(disk_entry);
        if (manager->capacity == maximum) return false;
        size_t capacity = !manager->capacity ? 4 : manager->capacity > maximum / 2 ? maximum : manager->capacity * 2;
        disk_entry *entries = realloc(manager->entries, capacity * sizeof(disk_entry));
        if (!entries) return false;
        manager->entries = entries;
        manager->capacity = capacity;
    }
    disk_entry entry = {id, length, payload};
    size_t index = manager->length;
    while (index && compare(&entry, &manager->entries[index - 1]) < 0) {
        manager->entries[index] = manager->entries[index - 1];
        --index;
    }
    manager->entries[index] = entry;
    ++manager->length;
    return true;
}

void *ghostos_vm_disk_manager_remove(ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length) {
    size_t index = find(manager, id, length);
    if (index == manager->length) return NULL;
    void *payload = manager->entries[index].payload;
    --manager->length;
    if (index < manager->length) memmove(manager->entries + index, manager->entries + index + 1,
        (manager->length - index) * sizeof(disk_entry));
    return payload;
}
