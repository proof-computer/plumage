//! ADR-0272 §2: the committed UN calendar, compiled into every peer.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::day::Day;

pub const URL: &str = "https://www.un.org/en/observances/list-days-weeks";
pub const HOST: &str = "www.un.org";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Observance {
    pub month: u8,
    pub day: u8,
    pub name: String,
    pub resolutions: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Calendar {
    source: String,
    entries: Vec<Observance>,
}

pub fn calendar() -> &'static [Observance] {
    static CALENDAR: OnceLock<Calendar> = OnceLock::new();
    &CALENDAR
        .get_or_init(|| {
            let calendar: Calendar =
                serde_json::from_str(include_str!("../assets/un-observances.json"))
                    .expect("compiled UN calendar is valid");
            assert_eq!(calendar.source, URL, "compiled UN calendar source");
            calendar
        })
        .entries
}

pub fn for_day(day: &Day) -> Vec<&'static Observance> {
    calendar()
        .iter()
        .filter(|entry| (entry.month, entry.day) == day.month_day())
        .collect()
}

pub fn line(observance: &Observance) -> String {
    if observance.resolutions.is_empty() {
        observance.name.clone()
    } else {
        format!("{} ({})", observance.name, observance.resolutions)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn compiled_calendar_is_complete_and_clean() {
        let entries = calendar();
        assert!(entries.len() >= 200);
        let dates: BTreeSet<_> = entries.iter().map(|e| (e.month, e.day)).collect();
        assert!(dates.len() >= 180);
        for entry in entries {
            assert!((1..=12).contains(&entry.month));
            assert!(
                format!("2000-{:02}-{:02}", entry.month, entry.day)
                    .parse::<Day>()
                    .is_ok()
            );
            assert!(!entry.name.is_empty());
            assert_eq!(entry.name, entry.name.trim());
            assert!(!entry.name.contains(['\n', '<']));
            assert!(!entry.resolutions.contains('\n'));
            assert!(!(entry.resolutions.starts_with('(') && entry.resolutions.ends_with(')')));
        }
    }

    #[test]
    fn dated_entries_preserve_names_resolutions_and_order() {
        let wetlands = for_day(&"2026-02-02".parse().unwrap());
        assert_eq!(wetlands.len(), 1);
        assert_eq!(wetlands[0].name, "World Wetlands Day");
        assert_eq!(wetlands[0].resolutions, "A/RES/75/317");
        assert_eq!(line(wetlands[0]), "World Wetlands Day (A/RES/75/317)");
        assert!(for_day(&"2026-02-01".parse().unwrap()).iter().any(|e| {
            e.name.starts_with("World Interfaith Harmony Week")
                && e.name.contains("February")
                && e.resolutions == "A/RES/65/5"
        }));
        assert!(
            for_day(&"2026-03-22".parse().unwrap())
                .iter()
                .any(|e| e.name == "World Water Day")
        );
        let un_day = for_day(&"2026-10-24".parse().unwrap());
        assert!(un_day.len() >= 2);
        assert!(un_day.iter().any(|e| e.name == "United Nations Day"));
        for day in ["2026-10-26", "2026-11-01", "2028-02-29"] {
            assert!(for_day(&day.parse().unwrap()).is_empty());
        }
        let march = for_day(&"2026-03-21".parse().unwrap());
        let in_file: Vec<_> = calendar()
            .iter()
            .filter(|e| e.month == 3 && e.day == 21)
            .collect();
        assert_eq!(march, in_file);
        let no_resolution = calendar()
            .iter()
            .find(|e| e.resolutions.is_empty())
            .unwrap();
        assert_eq!(line(no_resolution), no_resolution.name);
    }
}
