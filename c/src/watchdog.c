#include "ghostos/watchdog.h"

#include <stdatomic.h>

static _Atomic uint32_t service_ready;
static _Atomic uint64_t service_heartbeats[GHOSTOS_WATCHDOG_SERVICE_CAPACITY];
static _Atomic uint64_t service_sequences[GHOSTOS_WATCHDOG_SERVICE_CAPACITY];
static _Atomic uint32_t service_latched;
static _Atomic bool diagnostics_enabled;
static _Atomic uint64_t cpu_online[2];
static _Atomic uint64_t cpu_heartbeats[GHOSTOS_WATCHDOG_MAX_CPUS];
static _Atomic uint64_t cpu_latched[2];

void ghostos_watchdog_init(void) {
    atomic_store_explicit(&service_ready, 0, memory_order_relaxed);
    atomic_store_explicit(&service_latched, 0, memory_order_relaxed);
    atomic_store_explicit(&diagnostics_enabled, false, memory_order_relaxed);
    for (size_t index = 0; index < GHOSTOS_WATCHDOG_SERVICE_CAPACITY; ++index) {
        atomic_store_explicit(&service_heartbeats[index], 0, memory_order_relaxed);
        atomic_store_explicit(&service_sequences[index], 0, memory_order_relaxed);
    }
    for (size_t index = 0; index < 2; ++index) {
        atomic_store_explicit(&cpu_online[index], 0, memory_order_relaxed);
        atomic_store_explicit(&cpu_latched[index], 0, memory_order_relaxed);
    }
    for (size_t index = 0; index < GHOSTOS_WATCHDOG_MAX_CPUS; ++index) {
        atomic_store_explicit(&cpu_heartbeats[index], 0, memory_order_relaxed);
    }
}

void ghostos_watchdog_service_ready(size_t role, uint64_t now_us) {
    if (role >= GHOSTOS_WATCHDOG_SERVICE_CAPACITY) return;
    uint32_t bit = UINT32_C(1) << role;
    atomic_fetch_or_explicit(&service_ready, bit, memory_order_release);
    uint64_t expected = 0;
    (void)atomic_compare_exchange_strong_explicit(&service_heartbeats[role], &expected,
        now_us, memory_order_release, memory_order_relaxed);
}

void ghostos_watchdog_service_heartbeat(size_t role, uint64_t sequence, uint64_t now_us) {
    if (role >= GHOSTOS_WATCHDOG_SERVICE_CAPACITY || sequence == 0) return;
    uint32_t bit = UINT32_C(1) << role;
    if ((atomic_load_explicit(&service_ready, memory_order_acquire) & bit) == 0) return;
    atomic_store_explicit(&service_sequences[role], sequence, memory_order_relaxed);
    atomic_store_explicit(&service_heartbeats[role], now_us, memory_order_release);
    atomic_fetch_and_explicit(&service_latched, ~bit, memory_order_release);
}

void ghostos_watchdog_service_activity(size_t role, uint64_t now_us) {
    if (role >= GHOSTOS_WATCHDOG_SERVICE_CAPACITY) return;
    uint32_t bit = UINT32_C(1) << role;
    if ((atomic_load_explicit(&service_ready, memory_order_acquire) & bit) == 0) return;
    uint64_t expected = 0;
    (void)atomic_compare_exchange_strong_explicit(&service_sequences[role], &expected, 1,
        memory_order_relaxed, memory_order_relaxed);
    atomic_store_explicit(&service_heartbeats[role], now_us, memory_order_release);
    atomic_fetch_and_explicit(&service_latched, ~bit, memory_order_release);
}

uint64_t ghostos_watchdog_service_sequence(size_t role) {
    if (role >= GHOSTOS_WATCHDOG_SERVICE_CAPACITY) return 0;
    return atomic_load_explicit(&service_sequences[role], memory_order_acquire);
}

bool ghostos_watchdog_diagnostics_enabled(void) {
    return atomic_load_explicit(&diagnostics_enabled, memory_order_acquire);
}

void ghostos_watchdog_set_diagnostics_enabled(bool enabled) {
    atomic_store_explicit(&diagnostics_enabled, enabled, memory_order_release);
}

void ghostos_watchdog_cpu_online(uint8_t cpu, uint64_t now_us) {
    if (cpu >= GHOSTOS_WATCHDOG_MAX_CPUS) return;
    size_t word = cpu / 64;
    uint64_t bit = UINT64_C(1) << (cpu % 64);
    atomic_fetch_or_explicit(&cpu_online[word], bit, memory_order_release);
    atomic_store_explicit(&cpu_heartbeats[cpu], now_us, memory_order_release);
}

void ghostos_watchdog_cpu_tick(uint8_t cpu, uint64_t now_us) {
    if (cpu >= GHOSTOS_WATCHDOG_MAX_CPUS) return;
    size_t word = cpu / 64;
    uint64_t bit = UINT64_C(1) << (cpu % 64);
    atomic_store_explicit(&cpu_heartbeats[cpu], now_us, memory_order_release);
    atomic_fetch_and_explicit(&cpu_latched[word], ~bit, memory_order_release);
}

void ghostos_watchdog_cpu_offline(uint8_t cpu) {
    if (cpu >= GHOSTOS_WATCHDOG_MAX_CPUS) return;
    size_t word = cpu / 64;
    atomic_fetch_and_explicit(&cpu_online[word], ~(UINT64_C(1) << (cpu % 64)),
        memory_order_release);
}

static bool elapsed_exceeds(uint64_t now, uint64_t last, uint64_t timeout) {
    return (now >= last ? now - last : 0) > timeout;
}

ghostos_watchdog_report ghostos_watchdog_poll(uint64_t now_us) {
    ghostos_watchdog_report report = {0};
    uint32_t ready = atomic_load_explicit(&service_ready, memory_order_acquire);
    for (size_t role = 1; role < GHOSTOS_WATCHDOG_SERVICE_CAPACITY; ++role) {
        uint32_t bit = UINT32_C(1) << role;
        uint64_t last = atomic_load_explicit(&service_heartbeats[role], memory_order_acquire);
        if ((ready & bit) != 0 && last != 0 &&
            elapsed_exceeds(now_us, last, GHOSTOS_WATCHDOG_SERVICE_TIMEOUT_US) &&
            (atomic_fetch_or_explicit(&service_latched, bit, memory_order_acq_rel) & bit) == 0) {
            report.stale_services |= bit;
        }
    }
    for (size_t cpu = 0; cpu < GHOSTOS_WATCHDOG_MAX_CPUS; ++cpu) {
        size_t word = cpu / 64;
        uint64_t bit = UINT64_C(1) << (cpu % 64);
        if ((atomic_load_explicit(&cpu_online[word], memory_order_acquire) & bit) == 0) continue;
        uint64_t last = atomic_load_explicit(&cpu_heartbeats[cpu], memory_order_acquire);
        if (last != 0 && elapsed_exceeds(now_us, last, GHOSTOS_WATCHDOG_CPU_TIMEOUT_US) &&
            (atomic_fetch_or_explicit(&cpu_latched[word], bit, memory_order_acq_rel) & bit) == 0) {
            report.stale_cpus[word] |= bit;
        }
    }
    return report;
}
