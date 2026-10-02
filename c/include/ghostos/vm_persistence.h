#ifndef GHOSTOS_VM_PERSISTENCE_H
#define GHOSTOS_VM_PERSISTENCE_H

#include "ghostos/boot_protocol.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_PERSISTENCE_SECTOR_BYTES 512u
#define GHOSTOS_VM_PERSISTENCE_REGION_BYTES (64u * 1024u)
#define GHOSTOS_VM_PERSISTENCE_REGION_SECTORS 128u

typedef struct ghostos_vm_persistence ghostos_vm_persistence;
typedef struct {
    bool attached;
    bool (*read_sector)(void *context, uint64_t sector, uint8_t *bytes);
    bool (*write_sector)(void *context, uint64_t sector, const uint8_t *bytes);
    bool (*sync)(void *context);
    void *context;
} ghostos_vm_persistence_io;

ghostos_vm_persistence *ghostos_vm_persistence_new(void);
void ghostos_vm_persistence_free(ghostos_vm_persistence *state);
/* Attach: 0 success, 1 too-small disk, 2 storage I/O failure. Callbacks run
 * synchronously, are not retained, and must not reenter this port. */
uint8_t ghostos_vm_persistence_attach(ghostos_vm_persistence *state,
    uint64_t sectors, const ghostos_vm_persistence_io *io);
/* Port results: 0 success, 1 unsupported size, 2 invalid address, 3 not ready,
 * 4 read cursor outside the backing array (the old Rust slice-panic boundary).
 * Unknown reads return zero. Disk errors leave the command transition undone. */
uint8_t ghostos_vm_persistence_read(ghostos_vm_persistence *state,
    uint16_t port, uint8_t size, uint64_t *value);
uint8_t ghostos_vm_persistence_write(ghostos_vm_persistence *state,
    uint16_t port, uint64_t value, uint8_t size, const ghostos_vm_persistence_io *io);
void ghostos_vm_persistence_reset(ghostos_vm_persistence *state,
    const ghostos_vm_persistence_io *io);

#endif
