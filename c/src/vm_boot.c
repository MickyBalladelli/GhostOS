#include "ghostos/vm_boot.h"

static uint32_t read_le32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
        (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
}

bool ghostos_vm_boot_find_multiboot_header(const uint8_t *kernel, size_t length,
    uint32_t *offset) {
    if (!kernel || !offset) return false;
    size_t limit = length < 8192 ? length : 8192;
    if (limit < 12) return false;
    for (size_t index = 0; index <= limit - 12; index += 4) {
        uint32_t magic = read_le32(kernel + index);
        uint32_t flags = read_le32(kernel + index + 4);
        uint32_t checksum = read_le32(kernel + index + 8);
        if (magic == UINT32_C(0x1badb002) && magic + flags + checksum == 0) {
            *offset = (uint32_t)index;
            return true;
        }
    }
    return false;
}

bool ghostos_vm_boot_parse_multiboot(const uint8_t *header, size_t length,
    ghostos_vm_multiboot_info *info) {
    if (!header || !info || length < 28) return false;
    info->flags = read_le32(header);
    info->mem_lower = read_le32(header + 4);
    info->mem_upper = read_le32(header + 8);
    info->boot_device = read_le32(header + 12);
    info->cmdline = read_le32(header + 16);
    info->mods_count = read_le32(header + 20);
    info->mods_addr = read_le32(header + 24);
    return true;
}
