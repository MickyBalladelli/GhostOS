#ifndef GHOSTOS_TIME_H
#define GHOSTOS_TIME_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_TIME_PIT_TICK_US UINT64_C(10000)

void ghostos_time_initialize(void);
void ghostos_time_advance_monotonic(uint64_t elapsed_us);
uint64_t ghostos_time_timer_tick(void);
uint64_t ghostos_time_monotonic_now_us(void);
bool ghostos_time_realtime_ready(void);
bool ghostos_time_realtime_now_ns(uint64_t *now_ns);

#endif
