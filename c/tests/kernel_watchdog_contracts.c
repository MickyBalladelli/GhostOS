#include "ghostos/watchdog.h"

#include <assert.h>

int main(void) {
    ghostos_watchdog_init();
    assert(!ghostos_watchdog_diagnostics_enabled());
    ghostos_watchdog_set_diagnostics_enabled(true);
    assert(ghostos_watchdog_diagnostics_enabled());

    ghostos_watchdog_service_ready(3, 100);
    assert(ghostos_watchdog_service_sequence(3) == 0);
    ghostos_watchdog_service_activity(3, 200);
    assert(ghostos_watchdog_service_sequence(3) == 1);
    ghostos_watchdog_service_heartbeat(3, 9, 300);
    assert(ghostos_watchdog_service_sequence(3) == 9);

    ghostos_watchdog_report report = ghostos_watchdog_poll(300 +
        GHOSTOS_WATCHDOG_SERVICE_TIMEOUT_US + 1);
    assert(report.stale_services == (UINT32_C(1) << 3));
    report = ghostos_watchdog_poll(300 + GHOSTOS_WATCHDOG_SERVICE_TIMEOUT_US + 2);
    assert(report.stale_services == 0);
    ghostos_watchdog_service_activity(3, 400);
    report = ghostos_watchdog_poll(400 + GHOSTOS_WATCHDOG_SERVICE_TIMEOUT_US + 1);
    assert(report.stale_services == (UINT32_C(1) << 3));

    ghostos_watchdog_cpu_online(65, 1000);
    report = ghostos_watchdog_poll(1000 + GHOSTOS_WATCHDOG_CPU_TIMEOUT_US + 1);
    assert(report.stale_cpus[0] == 0);
    assert(report.stale_cpus[1] == (UINT64_C(1) << 1));
    report = ghostos_watchdog_poll(1000 + GHOSTOS_WATCHDOG_CPU_TIMEOUT_US + 2);
    assert(report.stale_cpus[1] == 0);
    ghostos_watchdog_cpu_tick(65, 2000);
    report = ghostos_watchdog_poll(2000 + GHOSTOS_WATCHDOG_CPU_TIMEOUT_US + 1);
    assert(report.stale_cpus[1] == (UINT64_C(1) << 1));
    ghostos_watchdog_cpu_offline(65);
    report = ghostos_watchdog_poll(2000 + 2 * GHOSTOS_WATCHDOG_CPU_TIMEOUT_US);
    assert(report.stale_cpus[1] == 0);

    ghostos_watchdog_service_ready(GHOSTOS_WATCHDOG_SERVICE_CAPACITY, 0);
    ghostos_watchdog_service_heartbeat(3, 0, 0);
    ghostos_watchdog_cpu_online(GHOSTOS_WATCHDOG_MAX_CPUS, 0);
    ghostos_watchdog_cpu_tick(GHOSTOS_WATCHDOG_MAX_CPUS, 0);
    ghostos_watchdog_cpu_offline(GHOSTOS_WATCHDOG_MAX_CPUS);
    return 0;
}
