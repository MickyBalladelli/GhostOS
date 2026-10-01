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

static ghostos_power_error io_error(ghostos_power_io_error error) {
    return error == GHOSTOS_POWER_IO_INVALID_ADDRESS ? GHOSTOS_POWER_INVALID_ADDRESS :
        error == GHOSTOS_POWER_IO_UNSUPPORTED ? GHOSTOS_POWER_UNSUPPORTED : GHOSTOS_POWER_OK;
}

ghostos_power_error ghostos_power_enable_acpi(const ghostos_power_platform *platform, size_t spins) {
    if (!platform) return GHOSTOS_POWER_UNSUPPORTED;
    if (platform->reduced_hardware) return GHOSTOS_POWER_OK;
    if (!platform->pm1a_control.present) return GHOSTOS_POWER_UNSUPPORTED;
    uint64_t value = 0;
    ghostos_power_error error = io_error(ghostos_power_register_read(platform->pm1a_control.register_value, &value));
    if (error) return error;
    if (value & 1) return GHOSTOS_POWER_OK;
    if (!platform->smi_command_port || !platform->acpi_enable_value) return GHOSTOS_POWER_UNSUPPORTED;
    ghostos_power_register command = {1, 8, 0, 1, platform->smi_command_port};
    error = io_error(ghostos_power_register_write(command, platform->acpi_enable_value));
    if (error) return error;
    for (;;) {
        error = io_error(ghostos_power_register_read(platform->pm1a_control.register_value, &value));
        if (error) return error;
        if (value & 1) return GHOSTOS_POWER_OK;
        if (!spins) return GHOSTOS_POWER_TIMED_OUT;
        --spins;
#if defined(__x86_64__)
        __asm__ volatile("pause");
#endif
    }
}

static bool event_register(ghostos_power_register base, uint8_t half, ghostos_power_register *out) {
    if (UINT64_MAX - base.address < half) return false;
    base.address += half;
    base.bit_width = (uint8_t)(half * 8);
    *out = base;
    return true;
}

ghostos_power_error ghostos_power_poll_events(const ghostos_power_platform *platform, uint16_t *events) {
    if (!platform || !events) return GHOSTOS_POWER_UNSUPPORTED;
    *events = 0;
    uint8_t half = platform->pm1_event_bytes / 2;
    if ((platform->pm1a_event.present || platform->pm1b_event.present) && !half) return GHOSTOS_POWER_MALFORMED;
    const ghostos_power_optional_register blocks[2] = {platform->pm1a_event, platform->pm1b_event};
    for (size_t index = 0; index < 2; ++index) {
        if (!blocks[index].present) continue;
        ghostos_power_register enable;
        if (!event_register(blocks[index].register_value, half, &enable)) return GHOSTOS_POWER_MALFORMED;
        ghostos_power_register status = blocks[index].register_value;
        status.bit_width = (uint8_t)(half * 8);
        uint64_t status_value = 0, enabled = 0;
        ghostos_power_error error = io_error(ghostos_power_register_read(status, &status_value));
        if (error) return error;
        error = io_error(ghostos_power_register_read(enable, &enabled));
        if (error) return error;
        uint16_t active = (uint16_t)status_value & (uint16_t)enabled;
        *events |= active;
        if (active) {
            error = io_error(ghostos_power_register_write(status, active));
            if (error) return error;
        }
    }
    return GHOSTOS_POWER_OK;
}

ghostos_power_error ghostos_power_set_event(const ghostos_power_platform *platform, uint8_t event, bool enabled) {
    if (!platform) return GHOSTOS_POWER_UNSUPPORTED;
    uint16_t mask;
    switch (event) {
        case GHOSTOS_POWER_EVENT_BUTTON: mask = UINT16_C(1) << 8; break;
        case GHOSTOS_POWER_EVENT_SLEEP: mask = UINT16_C(1) << 9; break;
        case GHOSTOS_POWER_EVENT_RTC: mask = UINT16_C(1) << 10; break;
        case GHOSTOS_POWER_EVENT_PCIE: mask = UINT16_C(1) << 14; break;
        case GHOSTOS_POWER_EVENT_WAKE: mask = UINT16_C(1) << 15; break;
        default: return GHOSTOS_POWER_UNSUPPORTED;
    }
    uint8_t half = platform->pm1_event_bytes / 2;
    if ((platform->pm1a_event.present || platform->pm1b_event.present) && !half) return GHOSTOS_POWER_MALFORMED;
    const ghostos_power_optional_register blocks[2] = {platform->pm1a_event, platform->pm1b_event};
    bool configured = false;
    for (size_t index = 0; index < 2; ++index) {
        if (!blocks[index].present) continue;
        ghostos_power_register reg;
        if (!event_register(blocks[index].register_value, half, &reg)) return GHOSTOS_POWER_MALFORMED;
        uint64_t previous = 0;
        ghostos_power_error error = io_error(ghostos_power_register_read(reg, &previous));
        if (error) return error;
        error = io_error(ghostos_power_register_write(reg, enabled ? previous | mask : previous & ~(uint64_t)mask));
        if (error) return error;
        configured = true;
    }
    return configured ? GHOSTOS_POWER_OK : GHOSTOS_POWER_UNSUPPORTED;
}

static ghostos_power_error write_sleep_control(ghostos_power_optional_register optional, uint8_t type) {
    if (!optional.present) return GHOSTOS_POWER_UNSUPPORTED;
    uint64_t previous = 0;
    ghostos_power_error error = io_error(ghostos_power_register_read(optional.register_value, &previous));
    if (error) return error;
    uint64_t value = (previous & ~(UINT64_C(7) << 10)) | ((uint64_t)(type & 7) << 10) | (UINT64_C(1) << 13);
    return io_error(ghostos_power_register_write(optional.register_value, value));
}

static ghostos_power_error write_sleep_state(const ghostos_power_platform *platform, uint8_t type_a, uint8_t type_b) {
    if (platform->reduced_hardware) {
        if (!platform->sleep_control.present) return GHOSTOS_POWER_UNSUPPORTED;
        uint64_t value = ((uint64_t)(type_a & 7) << 2) | (UINT64_C(1) << 5);
        return io_error(ghostos_power_register_write(platform->sleep_control.register_value, value));
    }
    ghostos_power_error error = write_sleep_control(platform->pm1a_control, type_a);
    if (error) return error;
    if (platform->pm1b_control.present) return write_sleep_control(platform->pm1b_control, type_b);
    return GHOSTOS_POWER_OK;
}

ghostos_power_error ghostos_power_request(const ghostos_power_platform *platform, uint8_t state) {
    if (!platform) return GHOSTOS_POWER_UNSUPPORTED;
    if (state == 2) {
        return platform->reset_present ? io_error(ghostos_power_register_write(platform->reset_register, platform->reset_value)) :
            GHOSTOS_POWER_UNSUPPORTED;
    }
    bool present = state == 0 ? platform->suspend_present : state == 1 && platform->soft_off_present;
    if (!present) return GHOSTOS_POWER_UNSUPPORTED;
    uint8_t type_a = state == 0 ? platform->suspend_a : platform->soft_off_a;
    uint8_t type_b = state == 0 ? platform->suspend_b : platform->soft_off_b;
    return write_sleep_state(platform, type_a, type_b);
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
