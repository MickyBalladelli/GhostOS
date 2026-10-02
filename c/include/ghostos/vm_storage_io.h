#ifndef GHOSTOS_VM_STORAGE_IO_H
#define GHOSTOS_VM_STORAGE_IO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Synchronous host adapters. Callbacks and context are never retained and
 * must not reenter the controller. False sector_count means no disk. */
typedef struct {
    bool (*read_memory)(void *, uint64_t, uint8_t *, size_t);
    bool (*write_memory)(void *, uint64_t, const uint8_t *, size_t);
    bool (*validate_dma)(void *, uint64_t, size_t, uint64_t);
    bool (*sector_count)(void *, uint64_t *);
    bool (*read_sector)(void *, uint64_t, uint8_t [512]);
    bool (*write_sector)(void *, uint64_t, const uint8_t [512]);
    bool (*flush_disk)(void *);
    void (*interrupt)(void *);
    void *context;
} ghostos_vm_storage_io;

#endif
