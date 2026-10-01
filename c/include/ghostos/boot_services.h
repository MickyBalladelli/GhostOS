#ifndef GHOSTOS_BOOT_SERVICES_H
#define GHOSTOS_BOOT_SERVICES_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_BOOT_SERVICE_COUNT 13u
#define GHOSTOS_BOOT_SERVICE_MAX_DEPENDENCIES 5u

enum {
    GHOSTOS_BOOT_SERVICE_FILESYSTEM = 0x46534444u,
    GHOSTOS_BOOT_SERVICE_STORAGE = 0x53544f52u,
    GHOSTOS_BOOT_SERVICE_NETWORK = 0x4e455444u,
    GHOSTOS_BOOT_SERVICE_LOGGING = 0x4c4f4744u,
    GHOSTOS_BOOT_SERVICE_AUDIT = 0x41554454u,
    GHOSTOS_BOOT_SERVICE_AUTHENTICATION = 0x41555448u,
    GHOSTOS_BOOT_SERVICE_PACKAGE = 0x504b4744u,
    GHOSTOS_BOOT_SERVICE_SHELL = 0x5348454cu,
    GHOSTOS_BOOT_SERVICE_LOGIN = 0x4c4f4749u,
    GHOSTOS_BOOT_SERVICE_PCI = 0x50434944u,
    GHOSTOS_BOOT_SERVICE_AHCI = 0x41484349u,
    GHOSTOS_BOOT_SERVICE_NVME = 0x4e564d45u,
    GHOSTOS_BOOT_SERVICE_ETHERNET = 0x45544844u
};

typedef enum {
    GHOSTOS_SERVICE_SYSTEM = 1,
    GHOSTOS_SERVICE_STORAGE_DRIVER = 2,
    GHOSTOS_SERVICE_NETWORK_DRIVER = 3
} ghostos_service_kind;

typedef struct {
    uint32_t id;
    uint64_t process_id;
    uint64_t image_id_low;
    uint64_t image_id_high;
    uint64_t capability_profile;
    const char *name;
    ghostos_service_kind kind;
    uint16_t max_restarts;
    uint64_t restart_window_us;
    uint64_t initial_backoff_us;
    uint64_t max_backoff_us;
    uint8_t dependency_count;
    uint32_t dependencies[GHOSTOS_BOOT_SERVICE_MAX_DEPENDENCIES];
} ghostos_boot_service_spec;

typedef struct {
    uint32_t service_id;
    uint64_t process_id;
    size_t startup_order;
    size_t dependency_count;
    uint32_t blocked_on;
    bool has_blocked_on;
    bool ready;
} ghostos_boot_startup_diagnostic;

const ghostos_boot_service_spec *ghostos_boot_service_specs(size_t *count);
const ghostos_boot_service_spec *ghostos_boot_service_find(uint32_t service_id);
bool ghostos_boot_service_startup_order(uint32_t *service_ids, size_t capacity,
                                        size_t *count);
bool ghostos_boot_process_for_service(uint32_t service_id, uint64_t *process_id);

#endif
