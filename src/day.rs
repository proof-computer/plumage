//! UTC calendar days. The date is the day's only input, and the arithmetic is
//! civil (days-from-civil), with no date crate.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The owner's close: 23:00:00.000 UTC on the open day.
pub const CLOSE_AFTER_MS: u64 = 23 * 3_600_000;

const DAY_MS: u64 = 86_400_000;

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A real UTC calendar date.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Day {
    year: i64,
    month: u8,
    day: u8,
}

impl Day {
    /// The UTC date that contains `unix_ms`.
    pub fn from_ms(unix_ms: u64) -> Day {
        let days = i64::try_from(unix_ms / DAY_MS).expect("unix day fits in i64");
        civil_from_days(days)
    }

    /// 00:00:00.000 UTC, as Unix milliseconds.
    ///
    /// The day is on or after 1970-01-01, which is every value [`FromStr`] accepts.
    pub fn open_ms(&self) -> u64 {
        let days = days_from_civil(self.year, i64::from(self.month), i64::from(self.day));
        u64::try_from(days).expect("UTC day is on or after 1970-01-01") * DAY_MS
    }

    /// 23:00:00.000 UTC, `open_ms + CLOSE_AFTER_MS`.
    pub fn close_ms(&self) -> u64 {
        self.open_ms() + CLOSE_AFTER_MS
    }

    /// The next civil date, including across months and leap days.
    pub fn next(&self) -> Day {
        self.shift(1)
    }

    /// The previous civil date, including across months and leap days.
    pub fn previous(&self) -> Day {
        self.shift(-1)
    }

    /// `(month, day)`, both numbered from 1.
    pub fn month_day(&self) -> (u8, u8) {
        (self.month, self.day)
    }

    /// Monday is 0 and Sunday is 6. 1970-01-01, day zero of the civil count, is Thursday.
    pub fn weekday(&self) -> u8 {
        let days = days_from_civil(self.year, i64::from(self.month), i64::from(self.day));
        u8::try_from((days + 3).rem_euclid(7)).expect("weekday is 0..=6")
    }

    /// `Monday` through `Sunday`, matching [`Self::weekday`].
    pub fn weekday_name(&self) -> &'static str {
        WEEKDAYS[usize::from(self.weekday())]
    }

    /// `January` through `December`.
    pub fn month_name(&self) -> &'static str {
        MONTHS[usize::from(self.month) - 1]
    }

    /// The first civil day of this month.
    pub fn first_of_month(&self) -> Day {
        self.shift(1 - i64::from(self.day))
    }

    fn shift(&self, by: i64) -> Day {
        let days = days_from_civil(self.year, i64::from(self.month), i64::from(self.day)) + by;
        civil_from_days(days)
    }
}

/// `HH:MM:SS` of the UTC time of day. Milliseconds are truncated.
pub fn utc_time(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) % 86_400;
    let hour = secs / 3_600;
    let minute = (secs % 3_600) / 60;
    let second = secs % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

impl fmt::Display for Day {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// `YYYY-MM-DD` was not a real civil date from 1970-01-01 through 9999-12-31.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseDayError;

impl fmt::Display for ParseDayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid UTC day")
    }
}

impl std::error::Error for ParseDayError {}

impl FromStr for Day {
    type Err = ParseDayError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(ParseDayError);
        }
        if !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
        {
            return Err(ParseDayError);
        }
        let year: i64 = text[..4].parse().map_err(|_| ParseDayError)?;
        let month: u8 = text[5..7].parse().map_err(|_| ParseDayError)?;
        let day: u8 = text[8..].parse().map_err(|_| ParseDayError)?;
        if !(1970..=9999).contains(&year) || !is_real_date(year, month, day) {
            return Err(ParseDayError);
        }
        Ok(Day { year, month, day })
    }
}

impl Serialize for Day {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Day {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

fn is_real_date(year: i64, month: u8, day: u8) -> bool {
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last).contains(&day)
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Howard Hinnant's `days_from_civil`. `z` is days since 1970-01-01.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = u64::try_from(year - era * 400).expect("year of era");
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * u64::try_from(shifted).expect("month") + 2) / 5
        + u64::try_from(day).expect("day")
        - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + i64::try_from(doe).expect("day of era") - 719468
}

fn civil_from_days(days: i64) -> Day {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = u64::try_from(z - era * 146097).expect("day of era");
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = i64::try_from(yoe).expect("year of era") + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    year += i64::from(month <= 2);
    Day {
        year,
        month: u8::try_from(month).expect("month"),
        day: u8::try_from(day).expect("day"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_ms_open_close_and_clock() {
        let open = 1_761_436_800_000;
        let day = Day::from_ms(open);
        assert_eq!(day.to_string(), "2025-10-26");
        assert_eq!(day.open_ms(), open);
        assert_eq!(day.close_ms(), open + CLOSE_AFTER_MS);
        assert_eq!(day.close_ms() - day.open_ms(), 23 * 3_600_000);
        assert_eq!(utc_time(open + 3_723_000), "01:02:03");
        assert_eq!(Day::from_ms(0).to_string(), "1970-01-01");
        assert_eq!(day.month_day(), (10, 26));
    }

    #[test]
    fn real_dates_round_trip_and_leap_days_do_not() {
        let day: Day = "2025-10-26".parse().unwrap();
        assert_eq!(day.to_string(), "2025-10-26");
        let json = serde_json::to_string(&day).unwrap();
        assert_eq!(json, "\"2025-10-26\"");
        assert_eq!(serde_json::from_str::<Day>(&json).unwrap(), day);

        assert!("2026-02-29".parse::<Day>().is_err());
        assert!("1969-12-31".parse::<Day>().is_err());
        assert!("2100-02-29".parse::<Day>().is_err());
        assert!("1970-1-01".parse::<Day>().is_err());
        let leap: Day = "2028-02-29".parse().unwrap();
        assert_eq!(leap.next().to_string(), "2028-03-01");
        assert_eq!(leap.next().previous(), leap);
        let last: Day = "9999-12-31".parse().unwrap();
        assert_eq!(last.to_string(), "9999-12-31");
    }

    #[test]
    fn weekday_month_name_and_first_of_month() {
        let epoch: Day = "1970-01-01".parse().unwrap();
        assert_eq!(epoch.weekday(), 3);
        assert_eq!(epoch.weekday_name(), "Thursday");

        let monday: Day = "2026-10-26".parse().unwrap();
        assert_eq!(monday.weekday(), 0);
        assert_eq!(monday.weekday_name(), "Monday");
        assert_eq!(monday.month_name(), "October");

        let leap: Day = "2000-02-29".parse().unwrap();
        assert_eq!(leap.weekday(), 1);
        assert_eq!(leap.weekday_name(), "Tuesday");

        let february: Day = "2024-02-29".parse().unwrap();
        assert_eq!(february.first_of_month(), "2024-02-01".parse().unwrap());
    }
}
