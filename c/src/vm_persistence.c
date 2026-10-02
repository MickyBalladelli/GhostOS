#include "ghostos/vm_persistence.h"

#include <stdlib.h>
#include <string.h>

enum { MODE_IDLE, MODE_READ, MODE_WRITE };

_Static_assert(sizeof(ghostos_vm_persistence_io) == 40, "persistence I/O ABI");
_Static_assert(offsetof(ghostos_vm_persistence_io, read_sector) == 8, "persistence read callback offset");
_Static_assert(offsetof(ghostos_vm_persistence_io, context) == 32, "persistence context offset");
_Static_assert(GHOSTOS_PERSISTENCE_MAX_BYTES + GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES <=
    GHOSTOS_VM_PERSISTENCE_REGION_BYTES, "persistence payload fits region");

struct ghostos_vm_persistence {
    uint64_t base_sector;
    uint8_t bytes[GHOSTOS_PERSISTENCE_MAX_BYTES];
    size_t length, cursor, expected_length;
    uint8_t mode;
};

static const uint8_t magic[] = "SYNOPS01";

static uint32_t read_u32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static void write_u32(uint8_t *bytes, uint32_t value) {
    for (unsigned i = 0; i < 4; ++i) bytes[i] = (uint8_t)(value >> (8 * i));
}

static uint32_t checksum(const uint8_t *bytes, size_t length) {
    uint32_t hash = UINT32_C(0x811c9dc5);
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= UINT32_C(0x01000193);
    }
    return hash;
}

ghostos_vm_persistence *ghostos_vm_persistence_new(void) {
    return calloc(1, sizeof(ghostos_vm_persistence));
}

void ghostos_vm_persistence_free(ghostos_vm_persistence *state) { free(state); }

static bool load_region(ghostos_vm_persistence *state, const ghostos_vm_persistence_io *io) {
    state->length = 0;
    memset(state->bytes, 0, sizeof(state->bytes));
    uint8_t region[GHOSTOS_VM_PERSISTENCE_REGION_BYTES] = {0};
    for (unsigned i = 0; i < GHOSTOS_VM_PERSISTENCE_REGION_SECTORS; ++i)
        if (!io->read_sector(io->context, state->base_sector + i,
            region + i * GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES)) return false;
    if (memcmp(region, magic, 8) != 0 || read_u32(region + 8) != 1) return true;
    size_t length = read_u32(region + 12);
    if (length > GHOSTOS_PERSISTENCE_MAX_BYTES) return true;
    const uint8_t *payload = region + GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES;
    if (checksum(payload, length) != read_u32(region + 16)) return true;
    memcpy(state->bytes, payload, length);
    state->length = length;
    return true;
}

uint8_t ghostos_vm_persistence_attach(ghostos_vm_persistence *state,
    uint64_t sectors, const ghostos_vm_persistence_io *io) {
    if (sectors < GHOSTOS_VM_PERSISTENCE_REGION_SECTORS) return 1;
    state->base_sector = sectors - GHOSTOS_VM_PERSISTENCE_REGION_SECTORS;
    state->length = state->cursor = 0;
    state->mode = MODE_IDLE;
    return load_region(state, io) ? 0 : 2;
}

static bool persist(ghostos_vm_persistence *state, const ghostos_vm_persistence_io *io) {
    if (!io->attached) return true;
    uint8_t region[GHOSTOS_VM_PERSISTENCE_REGION_BYTES] = {0};
    memcpy(region, magic, 8);
    write_u32(region + 8, 1);
    write_u32(region + 12, (uint32_t)state->length);
    write_u32(region + 16, checksum(state->bytes, state->length));
    memcpy(region + GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES, state->bytes, state->length);
    for (unsigned i = 0; i < GHOSTOS_VM_PERSISTENCE_REGION_SECTORS; ++i)
        if (!io->write_sector(io->context, state->base_sector + i,
            region + i * GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES)) return false;
    return io->sync(io->context);
}

static size_t advance_cursor(size_t cursor, size_t amount) {
    return cursor > SIZE_MAX - amount ? SIZE_MAX : cursor + amount;
}

uint8_t ghostos_vm_persistence_read(ghostos_vm_persistence *state,
    uint16_t port, uint8_t size, uint64_t *value) {
    *value = 0;
    if (port == GHOSTOS_PERSISTENCE_LENGTH_PORT && size == 4) *value = state->length;
    else if (port == GHOSTOS_PERSISTENCE_DATA_PORT && state->mode == MODE_READ) {
        if (size == 1) {
            if (state->cursor < sizeof(state->bytes)) *value = state->bytes[state->cursor];
            state->cursor = advance_cursor(state->cursor, 1);
        } else if (size == 4) {
            /* Rust permits an empty slice at the array end, then panics if
             * the cursor has moved beyond it. Preserve that adapter boundary
             * without constructing an out-of-bounds pointer in C. */
            if (state->cursor > sizeof(state->bytes)) return 4;
            size_t remaining = state->length > state->cursor ? state->length - state->cursor : 0;
            size_t count = remaining < 4 ? remaining : 4;
            uint8_t word[4] = {0};
            memcpy(word, state->bytes + state->cursor, count);
            *value = read_u32(word);
            state->cursor = advance_cursor(state->cursor, 4);
        }
    }
    return 0;
}

uint8_t ghostos_vm_persistence_write(ghostos_vm_persistence *state,
    uint16_t port, uint64_t raw, uint8_t size, const ghostos_vm_persistence_io *io) {
    if (port == GHOSTOS_PERSISTENCE_COMMAND_PORT && size == 1) {
        switch ((uint8_t)raw) {
            case GHOSTOS_PERSISTENCE_LOAD:
                if (io->attached && !load_region(state, io)) return 3;
                state->mode = MODE_READ;
                state->cursor = 0;
                return 0;
            case GHOSTOS_PERSISTENCE_SAVE:
                state->mode = MODE_WRITE;
                state->cursor = state->expected_length = state->length = 0;
                memset(state->bytes, 0, sizeof(state->bytes));
                return 0;
            case GHOSTOS_PERSISTENCE_FLUSH:
                if (!persist(state, io)) return 3;
                state->mode = MODE_IDLE;
                return 0;
            default: return 2;
        }
    }
    if (port == GHOSTOS_PERSISTENCE_LENGTH_PORT && size == 4) {
        size_t length = (uint32_t)raw;
        if (length > GHOSTOS_PERSISTENCE_MAX_BYTES) return 2;
        state->mode = MODE_WRITE;
        memset(state->bytes, 0, sizeof(state->bytes));
        state->expected_length = state->length = length;
        state->cursor = 0;
        return 0;
    }
    if (port == GHOSTOS_PERSISTENCE_DATA_PORT && (size == 1 || size == 4)) {
        if (state->cursor >= state->expected_length) return 2;
        size_t count = state->expected_length - state->cursor;
        if (count > size) count = size;
        uint8_t word[4];
        write_u32(word, (uint32_t)raw);
        memcpy(state->bytes + state->cursor, word, count);
        state->cursor = advance_cursor(state->cursor, size);
        return 0;
    }
    return 1;
}

void ghostos_vm_persistence_reset(ghostos_vm_persistence *state,
    const ghostos_vm_persistence_io *io) {
    (void)persist(state, io);
    state->mode = MODE_IDLE;
    state->cursor = 0;
}
