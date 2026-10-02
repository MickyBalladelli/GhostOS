#ifndef GHOSTOS_VM_DISK_MANAGEMENT_H
#define GHOSTOS_VM_DISK_MANAGEMENT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Controller: 0 AHCI, 1 NVMe, 2 virtio-blk. Role: 0 system, 1 data.
 * Persistence: 0 persistent, 1 copy-on-write, 2 disposable. */
typedef struct {
    const uint8_t *id;
    size_t id_length;
    uint32_t controller, role, persistence;
    uint8_t bus, slot;
    bool read_only, empty_id;
} ghostos_vm_disk_spec;

typedef struct {
    bool (*resolve)(void *context, size_t index);
    bool (*same_path)(void *context, size_t left, size_t right);
    void *context;
} ghostos_vm_disk_paths;

/* 0 success, 1 empty ID, 2 duplicate ID, 3 multiple system disks,
 * 4 unsupported location, 5 duplicate location, 6 host resolution failure,
 * 7 duplicate writable path. index receives the failing specification.
 * Path callbacks are synchronous and preserve native path equality. */
uint32_t ghostos_vm_disk_validate_specs(const ghostos_vm_disk_spec *specs,
    size_t count, const ghostos_vm_disk_paths *paths, size_t *index);
/* 0 success, 1 format mismatch, 2 invalid sector capacity,
 * 3 requested capacity mismatch. Format mismatch is checked first. */
uint32_t ghostos_vm_disk_validate_image(uint32_t format, uint64_t capacity,
    bool has_format, uint32_t expected_format, bool has_capacity, uint64_t expected_capacity);
bool ghostos_vm_disk_needs_clone(bool read_only, uint32_t persistence);
const char *ghostos_vm_disk_controller_name(uint32_t controller);
/* Returns bytes excluding NUL, or zero on invalid controller/short buffer. */
size_t ghostos_vm_disk_guest_id(uint32_t controller, uint8_t bus, uint8_t slot,
    char *output, size_t capacity);

typedef struct {
    /* Reserve: 0 created and closed, 1 already exists, 2 host error. */
    uint32_t (*reserve)(void *context, uint32_t attempt);
    bool (*copy)(void *context);
    void (*discard)(void *context);
    void *context;
} ghostos_vm_disk_clone_io;
/* 0 success, 1 host error (retained by callback), 2 exhausted 32 names.
 * On copy failure discard the reserved clone, retaining the copy error. */
uint32_t ghostos_vm_disk_clone_temporary(const ghostos_vm_disk_clone_io *io);

typedef struct ghostos_vm_disk_manager ghostos_vm_disk_manager;
/* The caller owns payload objects and immutable ID slices. Successful insert
 * transfers payload ownership to the manager. Remove transfers it back.
 * Free calls destroy for every remaining payload. */
ghostos_vm_disk_manager *ghostos_vm_disk_manager_new(void);
void ghostos_vm_disk_manager_free(ghostos_vm_disk_manager *manager,
    void (*destroy)(void *payload));
size_t ghostos_vm_disk_manager_length(const ghostos_vm_disk_manager *manager);
void *ghostos_vm_disk_manager_at(const ghostos_vm_disk_manager *manager, size_t index);
void *ghostos_vm_disk_manager_get(const ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length);
bool ghostos_vm_disk_manager_insert(ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length, void *payload);
void *ghostos_vm_disk_manager_remove(ghostos_vm_disk_manager *manager,
    const uint8_t *id, size_t length);

#endif
