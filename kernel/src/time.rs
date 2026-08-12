//! Kernel timekeeper shared by interrupt handling and user services.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const NANOS_PER_MICROSECOND: u64 = 1_000;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

static MONOTONIC_US: AtomicU64 = AtomicU64::new(0);
static REALTIME_BOOT_NS: AtomicU64 = AtomicU64::new(0);
static REALTIME_READY: AtomicBool = AtomicBool::new(false);

pub fn initialize() {
    MONOTONIC_US.store(0, Ordering::Release);
    if let Some(seconds) = platform::read_rtc_unix_seconds() {
        REALTIME_BOOT_NS.store(seconds.saturating_mul(NANOS_PER_SECOND), Ordering::Release);
        REALTIME_READY.store(true, Ordering::Release)
    }
}

pub fn advance_monotonic(elapsed_us: u64) {
    let _ = MONOTONIC_US.fetch_update(
        Ordering::AcqRel,
        Ordering::Acquire,
        |now| Some(now.saturating_add(elapsed_us)),
    );
}

pub fn monotonic_now_us() -> u64 {
    MONOTONIC_US.load(Ordering::Acquire)
}

pub fn realtime_ready() -> bool {
    REALTIME_READY.load(Ordering::Acquire)
}

pub fn realtime_now_ns() -> Option<u64> {
    if !realtime_ready() {
        return None
    }
    Some(
        REALTIME_BOOT_NS
            .load(Ordering::Acquire)
            .saturating_add(monotonic_now_us().saturating_mul(NANOS_PER_MICROSECOND)),
    )
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
mod platform {
    use core::arch::asm;

    #[derive(Clone, Copy, Eq, PartialEq)]
    struct RtcSample {
        second: u8,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        year: u16,
    }

    pub fn read_rtc_unix_seconds() -> Option<u64> {
        for _ in 0..8 {
            wait_for_update()?;
            let first = read_sample()?;
            wait_for_update()?;
            let second = read_sample()?;
            if first == second {
                return unix_seconds(first)
            }
        }
        None
    }

    fn wait_for_update() -> Option<()> {
        for _ in 0..100_000 {
            if read_register(0x0a) & 0x80 == 0 {
                return Some(())
            }
            core::hint::spin_loop()
        }
        None
    }

    fn read_sample() -> Option<RtcSample> {
        let status_b = read_register(0x0b);
        let binary = status_b & 0x04 != 0;
        let twenty_four_hour = status_b & 0x02 != 0;
        let second = convert(read_register(0x00), binary);
        let minute = convert(read_register(0x02), binary);
        let raw_hour = read_register(0x04);
        let pm = raw_hour & 0x80 != 0;
        let mut hour = convert(raw_hour & 0x7f, binary);
        if !twenty_four_hour {
            hour %= 12;
            if pm {
                hour += 12
            }
        }
        let day = convert(read_register(0x07), binary);
        let month = convert(read_register(0x08), binary);
        let short_year = convert(read_register(0x09), binary) as u16;
        let century = convert(read_register(0x32), binary) as u16;
        let year = if (19..=99).contains(&century) {
            century * 100 + short_year
        } else if short_year >= 70 {
            1900 + short_year
        } else {
            2000 + short_year
        };
        let sample = RtcSample {
            second,
            minute,
            hour,
            day,
            month,
            year,
        };
        validate(sample).then_some(sample)
    }

    fn convert(value: u8, binary: bool) -> u8 {
        if binary {
            value
        } else {
            (value & 0x0f).saturating_add((value >> 4).saturating_mul(10))
        }
    }

    fn validate(sample: RtcSample) -> bool {
        if sample.year < 1970
            || sample.month == 0
            || sample.month > 12
            || sample.day == 0
            || sample.hour > 23
            || sample.minute > 59
            || sample.second > 59
        {
            return false
        }
        let days = [
            31,
            if leap_year(sample.year) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        sample.day <= days[sample.month as usize - 1]
    }

    fn leap_year(year: u16) -> bool {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    }

    fn unix_seconds(sample: RtcSample) -> Option<u64> {
        let mut year = i64::from(sample.year);
        let month = i64::from(sample.month);
        year -= i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let shifted_month = month + if month > 2 { -3 } else { 9 };
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(sample.day) - 1;
        let day_of_era =
            year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        let days = era * 146_097 + day_of_era - 719_468;
        let days = u64::try_from(days).ok()?;
        days.checked_mul(86_400)?
            .checked_add(u64::from(sample.hour) * 3_600)?
            .checked_add(u64::from(sample.minute) * 60)?
            .checked_add(u64::from(sample.second))
    }

    fn read_register(register: u8) -> u8 {
        let value: u8;
        unsafe {
            asm!(
                "out dx, al",
                in("dx") 0x70u16,
                in("al") register | 0x80,
                options(nomem, nostack, preserves_flags),
            );
            asm!(
                "in al, dx",
                in("dx") 0x71u16,
                out("al") value,
                options(nomem, nostack, preserves_flags),
            );
            asm!(
                "out dx, al",
                in("dx") 0x70u16,
                in("al") 0x0d_u8,
                options(nomem, nostack, preserves_flags),
            );
        }
        value
    }
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
mod platform {
    pub fn read_rtc_unix_seconds() -> Option<u64> {
        None
    }
}
