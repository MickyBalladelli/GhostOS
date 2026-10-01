#ifndef GHOSTOS_POWER_H
#define GHOSTOS_POWER_H

#include "ghostos/boot_protocol.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t address_space;
    uint8_t bit_width;
    uint8_t bit_offset;
    uint8_t access_size;
    uint64_t address;
} ghostos_power_register;

typedef struct {
    uint64_t start;
    uint64_t length;
    uint32_t kind;
    uint32_t attributes;
} ghostos_power_memory_region;

typedef enum {
    GHOSTOS_POWER_IO_OK = 0,
    GHOSTOS_POWER_IO_INVALID_ADDRESS = 1,
    GHOSTOS_POWER_IO_UNSUPPORTED = 2
} ghostos_power_io_error;

bool ghostos_power_acpi_read(const ghostos_power_memory_region *regions, size_t region_count,
    uint64_t physical_offset, uint64_t physical_address, uint8_t *destination, size_t length);
ghostos_power_io_error ghostos_power_register_access_bytes(ghostos_power_register reg, uint8_t *bytes);
uint64_t ghostos_power_extract_field(uint64_t raw, ghostos_power_register reg);
ghostos_power_io_error ghostos_power_register_read(ghostos_power_register reg, uint64_t *value);
ghostos_power_io_error ghostos_power_register_write(ghostos_power_register reg, uint64_t value);
void ghostos_power_vm_shutdown(void);
void ghostos_power_vm_reboot(void);
void ghostos_power_reboot_fallback(void);

#endif
