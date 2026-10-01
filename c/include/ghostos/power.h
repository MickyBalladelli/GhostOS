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

typedef struct {
    bool present;
    ghostos_power_register register_value;
} ghostos_power_optional_register;

typedef struct {
    ghostos_power_optional_register pm1a_event, pm1b_event;
    ghostos_power_optional_register pm1a_control, pm1b_control;
    uint8_t pm1_event_bytes;
    uint32_t smi_command_port;
    uint8_t acpi_enable_value;
    bool reset_present;
    ghostos_power_register reset_register;
    uint8_t reset_value;
    bool reduced_hardware;
    ghostos_power_optional_register sleep_control, sleep_status;
    bool suspend_present, soft_off_present;
    uint8_t suspend_a, suspend_b, soft_off_a, soft_off_b;
} ghostos_power_platform;

typedef enum {
    GHOSTOS_POWER_OK = 0,
    GHOSTOS_POWER_INVALID_ADDRESS = 1,
    GHOSTOS_POWER_UNSUPPORTED = 2,
    GHOSTOS_POWER_TIMED_OUT = 3,
    GHOSTOS_POWER_MALFORMED = 4
} ghostos_power_error;

enum {
    GHOSTOS_POWER_EVENT_BUTTON = 1,
    GHOSTOS_POWER_EVENT_SLEEP = 2,
    GHOSTOS_POWER_EVENT_RTC = 4,
    GHOSTOS_POWER_EVENT_PCIE = 8,
    GHOSTOS_POWER_EVENT_WAKE = 16
};

bool ghostos_power_acpi_read(const ghostos_power_memory_region *regions, size_t region_count,
    uint64_t physical_offset, uint64_t physical_address, uint8_t *destination, size_t length);
ghostos_power_io_error ghostos_power_register_access_bytes(ghostos_power_register reg, uint8_t *bytes);
uint64_t ghostos_power_extract_field(uint64_t raw, ghostos_power_register reg);
ghostos_power_io_error ghostos_power_register_read(ghostos_power_register reg, uint64_t *value);
ghostos_power_io_error ghostos_power_register_write(ghostos_power_register reg, uint64_t value);
ghostos_power_error ghostos_power_enable_acpi(const ghostos_power_platform *platform, size_t spin_limit);
ghostos_power_error ghostos_power_poll_events(const ghostos_power_platform *platform, uint16_t *events);
ghostos_power_error ghostos_power_set_event(const ghostos_power_platform *platform, uint8_t event, bool enabled);
ghostos_power_error ghostos_power_request(const ghostos_power_platform *platform, uint8_t state);
void ghostos_power_vm_shutdown(void);
void ghostos_power_vm_reboot(void);
void ghostos_power_reboot_fallback(void);

#endif
