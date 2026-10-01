#ifndef GHOSTOS_VM_BOOT_H
#define GHOSTOS_VM_BOOT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint32_t flags;
    uint32_t mem_lower;
    uint32_t mem_upper;
    uint32_t boot_device;
    uint32_t cmdline;
    uint32_t mods_count;
    uint32_t mods_addr;
} ghostos_vm_multiboot_info;

bool ghostos_vm_boot_find_multiboot_header(const uint8_t *kernel, size_t length,
    uint32_t *offset);
bool ghostos_vm_boot_parse_multiboot(const uint8_t *header, size_t length,
    ghostos_vm_multiboot_info *info);

#endif
