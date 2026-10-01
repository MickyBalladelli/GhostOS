#include "ghostos/power.h"

bool ghostos_power_acpi_read(const ghostos_power_memory_region *regions, size_t count,
    uint64_t offset, uint64_t address, uint8_t *destination, size_t length) {
    if ((!regions && count) || !destination || count > GHOSTOS_MAX_MEMORY_REGIONS ||
        UINT64_MAX - address < length) return false;
    uint64_t end = address + length;
    bool mapped = false;
    for (size_t index = 0; index < count; ++index) {
        const ghostos_power_memory_region *region = &regions[index];
        if (region->kind == GHOSTOS_MEMORY_USABLE || UINT64_MAX - region->start < region->length) continue;
        uint64_t region_end = region->start + region->length;
        if (address >= region->start && end <= region_end) { mapped = true; break; }
    }
    if (!mapped || UINT64_MAX - address < offset) return false;
    uint64_t virtual_address = address + offset;
    if (virtual_address > UINTPTR_MAX) return false;
    const volatile uint8_t *source = (const volatile uint8_t *)(uintptr_t)virtual_address;
    for (size_t index = 0; index < length; ++index) destination[index] = source[index];
    return true;
}

ghostos_power_io_error ghostos_power_register_access_bytes(ghostos_power_register reg, uint8_t *bytes) {
    if (!bytes) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
    switch (reg.access_size) {
        case 1: *bytes = 1; break;
        case 2: *bytes = 2; break;
        case 3: *bytes = 4; break;
        case 4: *bytes = 8; break;
        case 0: {
            unsigned width = (unsigned)reg.bit_width + reg.bit_offset;
            *bytes = width <= 8 ? 1 : width <= 16 ? 2 : width <= 32 ? 4 : 8;
            break;
        }
        default: return GHOSTOS_POWER_IO_UNSUPPORTED;
    }
    if (!reg.address || reg.address % *bytes) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
    return GHOSTOS_POWER_IO_OK;
}

uint64_t ghostos_power_extract_field(uint64_t raw, ghostos_power_register reg) {
    if (reg.bit_offset >= 64) return 0;
    uint64_t shifted = raw >> reg.bit_offset;
    return reg.bit_width >= 64 ? shifted : shifted & ((UINT64_C(1) << reg.bit_width) - 1);
}

static uint64_t read_memory(uint64_t address, uint8_t bytes) {
    volatile uint8_t *p = (volatile uint8_t *)(uintptr_t)address;
    switch (bytes) {
        case 1: return *(volatile uint8_t *)p;
        case 2: return *(volatile uint16_t *)p;
        case 4: return *(volatile uint32_t *)p;
        default: return *(volatile uint64_t *)p;
    }
}

static void write_memory(uint64_t address, uint8_t bytes, uint64_t value) {
    volatile uint8_t *p = (volatile uint8_t *)(uintptr_t)address;
    switch (bytes) {
        case 1: *(volatile uint8_t *)p = (uint8_t)value; break;
        case 2: *(volatile uint16_t *)p = (uint16_t)value; break;
        case 4: *(volatile uint32_t *)p = (uint32_t)value; break;
        default: *(volatile uint64_t *)p = value; break;
    }
}

#if defined(__x86_64__)
static uint64_t read_port(uint16_t port, uint8_t bytes) {
    uint32_t value;
    if (bytes == 1) {
        uint8_t small;
        __asm__ volatile("inb %w1, %0" : "=a"(small) : "Nd"(port) : "memory");
        return small;
    }
    if (bytes == 2) {
        uint16_t small;
        __asm__ volatile("inw %w1, %0" : "=a"(small) : "Nd"(port) : "memory");
        return small;
    }
    __asm__ volatile("inl %w1, %0" : "=a"(value) : "Nd"(port) : "memory");
    return value;
}

static void write_port(uint16_t port, uint8_t bytes, uint64_t value) {
    if (bytes == 1) __asm__ volatile("outb %0, %w1" : : "a"((uint8_t)value), "Nd"(port) : "memory");
    else if (bytes == 2) __asm__ volatile("outw %0, %w1" : : "a"((uint16_t)value), "Nd"(port) : "memory");
    else __asm__ volatile("outl %0, %w1" : : "a"((uint32_t)value), "Nd"(port) : "memory");
}

static void out8(uint16_t port, uint8_t value) {
    __asm__ volatile("outb %0, %w1" : : "a"(value), "Nd"(port) : "memory");
}

static void out16(uint16_t port, uint16_t value) {
    __asm__ volatile("outw %0, %w1" : : "a"(value), "Nd"(port) : "memory");
}

static uint8_t in8(uint16_t port) {
    uint8_t value;
    __asm__ volatile("inb %w1, %0" : "=a"(value) : "Nd"(port) : "memory");
    return value;
}
#endif

ghostos_power_io_error ghostos_power_register_read(ghostos_power_register reg, uint64_t *value) {
    if (!value || reg.bit_offset >= 64) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
    uint8_t bytes;
    ghostos_power_io_error error = ghostos_power_register_access_bytes(reg, &bytes);
    if (error) return error;
    uint64_t raw;
    if (reg.address_space == 0) {
        if (reg.address > UINTPTR_MAX) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
        raw = read_memory(reg.address, bytes);
    } else if (reg.address_space == 1) {
#if defined(__x86_64__)
        if (reg.address > UINT16_MAX) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
        if (bytes == 8) return GHOSTOS_POWER_IO_UNSUPPORTED;
        raw = read_port((uint16_t)reg.address, bytes);
#else
        return GHOSTOS_POWER_IO_UNSUPPORTED;
#endif
    } else return GHOSTOS_POWER_IO_UNSUPPORTED;
    *value = ghostos_power_extract_field(raw, reg);
    return GHOSTOS_POWER_IO_OK;
}

ghostos_power_io_error ghostos_power_register_write(ghostos_power_register reg, uint64_t value) {
    if (reg.bit_offset >= 64) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
    uint8_t bytes;
    ghostos_power_io_error error = ghostos_power_register_access_bytes(reg, &bytes);
    if (error) return error;
    uint64_t shifted = value << reg.bit_offset;
    if (reg.address_space == 0) {
        if (reg.address > UINTPTR_MAX) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
        write_memory(reg.address, bytes, shifted);
    } else if (reg.address_space == 1) {
#if defined(__x86_64__)
        if (reg.address > UINT16_MAX) return GHOSTOS_POWER_IO_INVALID_ADDRESS;
        if (bytes == 8) return GHOSTOS_POWER_IO_UNSUPPORTED;
        write_port((uint16_t)reg.address, bytes, shifted);
#else
        return GHOSTOS_POWER_IO_UNSUPPORTED;
#endif
    } else return GHOSTOS_POWER_IO_UNSUPPORTED;
    return GHOSTOS_POWER_IO_OK;
}

void ghostos_power_vm_shutdown(void) {
#if defined(__x86_64__)
    out16(UINT16_C(0x0604), UINT16_C(0x3400));
#endif
}

void ghostos_power_vm_reboot(void) {
#if defined(__x86_64__)
    out16(UINT16_C(0x0604), UINT16_C(0x2000));
#endif
}

void ghostos_power_reboot_fallback(void) {
#if defined(__x86_64__)
    __asm__ volatile("cli" : : : "memory");
    for (size_t attempts = 100000; attempts && (in8(UINT16_C(0x0064)) & 2); --attempts) {
        __asm__ volatile("pause");
    }
    out8(UINT16_C(0x0064), UINT8_C(0xfe));
#endif
}
