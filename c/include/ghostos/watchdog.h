#ifndef GHOSTOS_WATCHDOG_H
#define GHOSTOS_WATCHDOG_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_WATCHDOG_SERVICE_CAPACITY 15u
#define GHOSTOS_WATCHDOG_SERVICE_TIMEOUT_US UINT64_C(5000000)
#define GHOSTOS_WATCHDOG_CPU_TIMEOUT_US UINT64_C(100000)
#define GHOSTOS_WATCHDOG_MAX_CPUS 128u

typedef struct {
    uint32_t stale_services;
    uint64_t stale_cpus[2];
} ghostos_watchdog_report;

void ghostos_watchdog_init(void);
void ghostos_watchdog_service_ready(size_t role, uint64_t now_us);
void ghostos_watchdog_service_heartbeat(size_t role, uint64_t sequence, uint64_t now_us);
void ghostos_watchdog_service_activity(size_t role, uint64_t now_us);
uint64_t ghostos_watchdog_service_sequence(size_t role);
bool ghostos_watchdog_diagnostics_enabled(void);
void ghostos_watchdog_set_diagnostics_enabled(bool enabled);
void ghostos_watchdog_cpu_online(uint8_t cpu, uint64_t now_us);
void ghostos_watchdog_cpu_tick(uint8_t cpu, uint64_t now_us);
void ghostos_watchdog_cpu_offline(uint8_t cpu);
ghostos_watchdog_report ghostos_watchdog_poll(uint64_t now_us);

#endif
