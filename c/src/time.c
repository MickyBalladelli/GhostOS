#include "ghostos/time.h"

#include <stddef.h>
#include <stdatomic.h>

#define NANOS_PER_MICROSECOND UINT64_C(1000)
#define NANOS_PER_SECOND UINT64_C(1000000000)

static atomic_uint_fast64_t monotonic_us = ATOMIC_VAR_INIT(0);
static atomic_uint_fast64_t realtime_boot_ns = ATOMIC_VAR_INIT(0);
static atomic_bool realtime_ready = ATOMIC_VAR_INIT(false);

#if defined(GHOSTOS_TIME_X86) && defined(__x86_64__)
typedef struct {
    uint8_t second, minute, hour, day, month;
    uint16_t year;
} rtc_sample;

static void write_cmos_register(uint8_t reg) {
    uint16_t port = 0x70;
    uint8_t value = reg | 0x80;
    __asm__ volatile("outb %0, %w1" : : "a"(value), "Nd"(port) : "memory");
}

static uint8_t read_cmos_data(void) {
    uint16_t port = 0x71;
    uint8_t value;
    __asm__ volatile("inb %w1, %0" : "=a"(value) : "Nd"(port) : "memory");
    return value;
}

static uint8_t read_register(uint8_t reg) {
    write_cmos_register(reg);
    uint8_t value = read_cmos_data();
    write_cmos_register(0x0d);
    return value;
}

static bool wait_for_update(void) {
    for (size_t count = 0; count < 100000; ++count) {
        if ((read_register(0x0a) & 0x80) == 0) return true;
        __asm__ volatile("pause");
    }
    return false;
}

static uint8_t convert_bcd(uint8_t value, bool binary) {
    if (binary) return value;
    return (uint8_t)((value & 0x0f) + (value >> 4) * 10u);
}

static bool leap_year(uint16_t year) {
    return year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
}

static bool sample_valid(rtc_sample sample) {
    if (sample.year < 1970 || sample.month == 0 || sample.month > 12 ||
        sample.day == 0 || sample.hour > 23 || sample.minute > 59 || sample.second > 59)
        return false;
    static const uint8_t days_by_month[12] = {
        31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31
    };
    uint8_t days = days_by_month[sample.month - 1];
    if (sample.month == 2 && leap_year(sample.year)) ++days;
    return sample.day <= days;
}

static bool read_sample(rtc_sample *out) {
    if (!out) return false;
    uint8_t status_b = read_register(0x0b);
    bool binary = (status_b & 0x04) != 0;
    bool twenty_four_hour = (status_b & 0x02) != 0;
    uint8_t second = convert_bcd(read_register(0x00), binary);
    uint8_t minute = convert_bcd(read_register(0x02), binary);
    uint8_t raw_hour = read_register(0x04);
    bool pm = (raw_hour & 0x80) != 0;
    uint8_t hour = convert_bcd(raw_hour & 0x7f, binary);
    if (!twenty_four_hour) {
        hour %= 12;
        if (pm) hour += 12;
    }
    uint8_t day = convert_bcd(read_register(0x07), binary);
    uint8_t month = convert_bcd(read_register(0x08), binary);
    uint16_t short_year = convert_bcd(read_register(0x09), binary);
    uint16_t century = convert_bcd(read_register(0x32), binary);
    uint16_t year = century >= 19 && century <= 99 ? century * 100 + short_year :
        (short_year >= 70 ? 1900 + short_year : 2000 + short_year);
    rtc_sample sample = {second, minute, hour, day, month, year};
    if (!sample_valid(sample)) return false;
    *out = sample;
    return true;
}

static bool unix_seconds(rtc_sample sample, uint64_t *out) {
    int64_t year = sample.year;
    int64_t month = sample.month;
    year -= month <= 2;
    int64_t era = year / 400;
    int64_t year_of_era = year - era * 400;
    int64_t shifted_month = month + (month > 2 ? -3 : 9);
    int64_t day_of_year = (153 * shifted_month + 2) / 5 + sample.day - 1;
    int64_t day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    int64_t days = era * 146097 + day_of_era - 719468;
    if (days < 0 || (uint64_t)days > UINT64_MAX / 86400) return false;
    uint64_t seconds = (uint64_t)days * 86400;
    seconds += (uint64_t)sample.hour * 3600 + (uint64_t)sample.minute * 60 + sample.second;
    *out = seconds;
    return true;
}

static bool read_rtc_unix_seconds(uint64_t *out) {
    for (size_t attempt = 0; attempt < 8; ++attempt) {
        rtc_sample first, second;
        if (!wait_for_update() || !read_sample(&first) ||
            !wait_for_update() || !read_sample(&second))
            return false;
        if (first.second == second.second && first.minute == second.minute &&
            first.hour == second.hour && first.day == second.day &&
            first.month == second.month && first.year == second.year)
            return unix_seconds(first, out);
    }
    return false;
}
#else
static bool read_rtc_unix_seconds(uint64_t *out) {
    (void)out;
    return false;
}
#endif

void ghostos_time_initialize(void) {
    atomic_store_explicit(&monotonic_us, 0, memory_order_release);
    uint64_t seconds;
    if (read_rtc_unix_seconds(&seconds)) {
        uint64_t boot_ns = seconds > UINT64_MAX / NANOS_PER_SECOND ? UINT64_MAX :
            seconds * NANOS_PER_SECOND;
        atomic_store_explicit(&realtime_boot_ns, boot_ns, memory_order_release);
        atomic_store_explicit(&realtime_ready, true, memory_order_release);
    }
}

void ghostos_time_advance_monotonic(uint64_t elapsed_us) {
    uint64_t now = atomic_load_explicit(&monotonic_us, memory_order_acquire);
    for (;;) {
        uint64_t next = UINT64_MAX - now < elapsed_us ? UINT64_MAX : now + elapsed_us;
        if (atomic_compare_exchange_weak_explicit(&monotonic_us, &now, next,
                memory_order_acq_rel, memory_order_acquire))
            return;
    }
}

uint64_t ghostos_time_monotonic_now_us(void) {
    return atomic_load_explicit(&monotonic_us, memory_order_acquire);
}

uint64_t ghostos_time_timer_tick(void) {
    ghostos_time_advance_monotonic(GHOSTOS_TIME_PIT_TICK_US);
    return GHOSTOS_TIME_PIT_TICK_US;
}

bool ghostos_time_realtime_ready(void) {
    return atomic_load_explicit(&realtime_ready, memory_order_acquire);
}

bool ghostos_time_realtime_now_ns(uint64_t *now_ns) {
    if (!now_ns || !ghostos_time_realtime_ready()) return false;
    uint64_t boot = atomic_load_explicit(&realtime_boot_ns, memory_order_acquire);
    uint64_t monotonic = ghostos_time_monotonic_now_us();
    uint64_t elapsed_ns = monotonic > UINT64_MAX / NANOS_PER_MICROSECOND ? UINT64_MAX :
        monotonic * NANOS_PER_MICROSECOND;
    *now_ns = UINT64_MAX - boot < elapsed_ns ? UINT64_MAX : boot + elapsed_ns;
    return true;
}
