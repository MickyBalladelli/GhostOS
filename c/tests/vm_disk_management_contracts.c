/* Policy portions of the two retained Rust disk-management cases.
 * Native filesystem locking and cleanup fixtures still require a full port. */
#include "ghostos/vm_disk_management.h"
#include "ghostos/vm_disk_image.h"

#include <assert.h>
#include <string.h>

typedef struct {
    uint8_t source[4096], clone[4096];
    uint8_t *active;
    uint64_t position;
    bool reserved, discarded;
} fixture;

static uint32_t reserve_clone(void *raw, uint32_t attempt) {
    fixture *state = raw;
    assert(attempt == 0);
    state->reserved = true;
    return 0;
}
static bool copy_clone(void *raw) {
    fixture *state = raw;
    assert(state->reserved);
    memcpy(state->clone, state->source, sizeof(state->clone));
    state->active = state->clone;
    return true;
}
static void discard_clone(void *raw) {
    fixture *state = raw;
    state->discarded = true;
}
static bool file_length(void *raw, uint64_t *length) {
    (void)raw;
    *length = 4096;
    return true;
}
static bool file_seek(void *raw, uint32_t origin, uint64_t offset, uint64_t *position) {
    fixture *state = raw;
    uint64_t base = origin == 0 ? 0 : origin == 1 ? 4096 : state->position;
    state->position = base + offset;
    if (state->position > 4096) return false;
    *position = state->position;
    return true;
}
static bool file_read(void *raw, uint8_t *bytes, size_t length) {
    fixture *state = raw;
    if (length > 4096 - state->position) return false;
    memcpy(bytes, state->active + state->position, length);
    state->position += length;
    return true;
}
static bool file_write(void *raw, const uint8_t *bytes, size_t length) {
    fixture *state = raw;
    if (length > 4096 - state->position) return false;
    memcpy(state->active + state->position, bytes, length);
    state->position += length;
    return true;
}
static bool file_resize(void *raw, uint64_t length) {
    (void)raw;
    return length == 4096;
}
static bool file_flush(void *raw) { (void)raw; return true; }
static ghostos_vm_disk_file_io file_io(fixture *state) {
    return (ghostos_vm_disk_file_io){file_length, file_seek, file_read,
        file_write, file_resize, file_flush, file_flush, state};
}

static void copy_on_write_and_disposable_isolate_guest_writes(void) {
    for (uint32_t persistence = 1; persistence <= 2; ++persistence) {
        fixture state = {0};
        state.active = state.source;
        assert(ghostos_vm_disk_needs_clone(false, persistence));
        ghostos_vm_disk_clone_io clone_io = {reserve_clone, copy_clone, discard_clone, &state};
        assert(ghostos_vm_disk_clone_temporary(&clone_io) == 0);
        ghostos_vm_disk_file_io io = file_io(&state);
        ghostos_vm_disk_error error = {0};
        ghostos_vm_disk_image *image = ghostos_vm_disk_image_open(&io, true, true, &error);
        assert(image);
        uint8_t sector[512];
        memset(sector, 0xd7, sizeof(sector));
        assert(ghostos_vm_disk_image_write(image, &io, 0, sector, &error));
        assert(ghostos_vm_disk_image_flush(&io, true, &error));
        ghostos_vm_disk_image_free(image);
        assert(state.clone[0] == 0xd7);
        uint8_t zero[4096] = {0};
        assert(!memcmp(state.source, zero, sizeof(zero)));
    }
}

static void read_only_attachment_isolated_from_writes(void) {
    fixture state = {0};
    state.active = state.source;
    assert(!ghostos_vm_disk_needs_clone(true, 0));
    ghostos_vm_disk_file_io io = file_io(&state);
    ghostos_vm_disk_error error = {0};
    ghostos_vm_disk_image *image = ghostos_vm_disk_image_open(&io, false, true, &error);
    assert(image);
    assert(!ghostos_vm_disk_image_writable(image));
    uint8_t sector[512];
    memset(sector, 0xe1, sizeof(sector));
    assert(!ghostos_vm_disk_image_write(image, &io, 0, sector, &error));
    assert(error.code == 2);
    ghostos_vm_disk_image_free(image);
    uint8_t zero[4096] = {0};
    assert(!memcmp(state.source, zero, sizeof(zero)));
}

int main(void) {
    copy_on_write_and_disposable_isolate_guest_writes();
    read_only_attachment_isolated_from_writes();
    return 0;
}
