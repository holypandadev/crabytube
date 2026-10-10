//! The local wall-clock time, for the Home greeting. The time zone is the system's.

/// Local time as the greeting needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClock {
    pub hour: u8,
    pub minute: u8,
    /// 0 is Sunday, as in C's `tm_wday`.
    pub weekday: u8,
    pub day: u8,
    /// 1 to 12.
    pub month: u8,
}

impl LocalClock {
    /// The current time in the system's time zone.
    pub fn now() -> Self {
        Self::from_zoned(&jiff::Zoned::now())
    }

    fn from_zoned(zoned: &jiff::Zoned) -> Self {
        Self {
            hour: zoned.hour() as u8,
            minute: zoned.minute() as u8,
            weekday: zoned.weekday().to_sunday_zero_offset() as u8,
            day: zoned.day() as u8,
            month: zoned.month() as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zoned_time_gives_its_wall_clock_fields() {
        // 9 October 2026 is a Friday: weekday 5 counted from Sunday.
        let zoned = jiff::civil::DateTime::new(2026, 10, 9, 21, 14, 0, 0)
            .and_then(|datetime| datetime.to_zoned(jiff::tz::TimeZone::UTC))
            .expect("a valid date and time");
        assert_eq!(
            LocalClock::from_zoned(&zoned),
            LocalClock {
                hour: 21,
                minute: 14,
                weekday: 5,
                day: 9,
                month: 10,
            }
        );
    }

    #[test]
    fn the_system_clock_reads_plausible_fields() {
        let now = LocalClock::now();
        assert!(now.hour < 24);
        assert!(now.minute < 60);
        assert!(now.weekday < 7);
        assert!((1..=31).contains(&now.day));
        assert!((1..=12).contains(&now.month));
    }
}
