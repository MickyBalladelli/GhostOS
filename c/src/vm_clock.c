#if defined(_WIN32)
#include <windows.h>
#else
#define _POSIX_C_SOURCE 200809L
#include <time.h>
#endif

#include "ghostos/vm_clock.h"
#include <stdlib.h>

struct ghostos_vm_host_clock {
    uint64_t started_ns;
};

struct ghostos_vm_manual_clock {
    uint64_t now_ns;
};

uint64_t ghostos_vm_clock_now_ns(void) {
#if defined(_WIN32)
    LARGE_INTEGER counter, frequency;
    if (!QueryPerformanceFrequency(&frequency) || !QueryPerformanceCounter(&counter) ||
        frequency.QuadPart <= 0 || counter.QuadPart < 0) return 0;
    uint64_t ticks = (uint64_t)counter.QuadPart;
    uint64_t rate = (uint64_t)frequency.QuadPart;
    return ticks / rate * UINT64_C(1000000000) +
        ticks % rate * UINT64_C(1000000000) / rate;
#else
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) != 0 || value.tv_sec < 0 || value.tv_nsec < 0)
        return 0;
    uint64_t seconds = (uint64_t)value.tv_sec;
    if (seconds > UINT64_MAX / UINT64_C(1000000000)) return UINT64_MAX;
    return seconds * UINT64_C(1000000000) + (uint64_t)value.tv_nsec;
#endif
}

uint64_t ghostos_vm_clock_elapsed_ns(uint64_t start_ns) {
    uint64_t now = ghostos_vm_clock_now_ns();
    return now >= start_ns ? now - start_ns : 0;
}

bool ghostos_vm_manual_clock_set(uint64_t *clock, uint64_t now_ns) {
    if (!clock || now_ns < *clock) return false;
    *clock = now_ns;
    return true;
}

uint64_t ghostos_vm_manual_clock_advance(uint64_t *clock, uint64_t elapsed_ns) {
    if (!clock) return 0;
    *clock = UINT64_MAX - *clock < elapsed_ns ? UINT64_MAX : *clock + elapsed_ns;
    return *clock;
}

ghostos_vm_host_clock *ghostos_vm_host_clock_new(void) {
    ghostos_vm_host_clock *clock = malloc(sizeof(*clock));
    if (clock) clock->started_ns = ghostos_vm_clock_now_ns();
    return clock;
}

void ghostos_vm_host_clock_free(ghostos_vm_host_clock *clock) {
    free(clock);
}

uint64_t ghostos_vm_host_clock_now(const ghostos_vm_host_clock *clock) {
    return clock ? ghostos_vm_clock_elapsed_ns(clock->started_ns) : 0;
}

ghostos_vm_manual_clock *ghostos_vm_manual_clock_new(uint64_t now_ns) {
    ghostos_vm_manual_clock *clock = malloc(sizeof(*clock));
    if (clock) clock->now_ns = now_ns;
    return clock;
}

void ghostos_vm_manual_clock_free(ghostos_vm_manual_clock *clock) {
    free(clock);
}

bool ghostos_vm_manual_clock_set_state(ghostos_vm_manual_clock *clock, uint64_t now_ns) {
    if (!clock) return false;
    return ghostos_vm_manual_clock_set(&clock->now_ns, now_ns);
}

uint64_t ghostos_vm_manual_clock_advance_state(ghostos_vm_manual_clock *clock,
    uint64_t elapsed_ns) {
    return clock ? ghostos_vm_manual_clock_advance(&clock->now_ns, elapsed_ns) : 0;
}

uint64_t ghostos_vm_manual_clock_now(const ghostos_vm_manual_clock *clock) {
    return clock ? clock->now_ns : 0;
}
