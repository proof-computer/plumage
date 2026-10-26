//! The days: a painted stripe, a Monday-first calendar, and days per colour.

use std::collections::BTreeMap;

use crate::colour::Colour;
use crate::day::Day;
use crate::tally::{DAYS_PER_COLOUR_RULE, days_per_colour};

use super::layout::{document, push_escaped};
use super::{ArchivePage, Paint, Section};

const HATCH: &str = "repeating-linear-gradient(45deg,#E4E3DE 0 6px,#F3F2EE 6px 12px)";

/// The days index at depth 1. Months run from the newest back to the oldest.
pub fn render_archive(page: &ArchivePage<'_>) -> String {
    let mut ordered: Vec<&(Day, Paint)> = page.days.iter().collect();
    ordered.sort_by_key(|(day, _)| *day);
    let recorded = recorded_days(page);
    let decided = ordered
        .iter()
        .filter(|(_, paint)| matches!(paint, Paint::Decided(_)))
        .count();
    let undecided = ordered
        .iter()
        .filter(|(_, paint)| matches!(paint, Paint::Undecided))
        .count();

    let mut body = String::from("<h1>The days</h1>\n<p>");
    body.push_str(
        "Every day the swarm has closed, in the colour it chose. A day with no colour held by more than half for an hour stays unpainted.",
    );
    body.push_str("</p>\n");
    push_stripe(&mut body, &ordered);
    body.push_str("<p><span>");
    if let (Some((first, _)), Some((last, _))) = (ordered.first(), ordered.last()) {
        body.push_str(&first.to_string());
        body.push_str("</span><span>");
        body.push_str(&count_line(ordered.len(), decided, undecided));
        body.push_str("</span><span>");
        body.push_str(&last.to_string());
        body.push_str("</span>");
    } else {
        body.push_str(&count_line(0, 0, 0));
    }
    body.push_str("</p>\n");

    let as_of = Day::from_ms(page.chrome.as_of_ms);
    for month in month_starts(page.days) {
        push_month(&mut body, month, &recorded, as_of);
    }
    push_tally(&mut body, page);
    document("The days", Some(Section::Days), 1, &page.chrome, &body)
}

fn recorded_days(page: &ArchivePage<'_>) -> BTreeMap<Day, Paint> {
    let mut recorded = BTreeMap::new();
    for (day, paint) in page.days {
        recorded.insert(*day, *paint);
    }
    recorded
}

fn push_stripe(out: &mut String, days: &[&(Day, Paint)]) {
    out.push_str("<div class=\"stripe\" role=\"img\" style=\"display:flex;height:72px\">");
    for (_, paint) in days {
        out.push_str("<span class=\"stripe-seg\" style=\"flex:1;background:");
        out.push_str(&paint_background(*paint));
        out.push_str("\"></span>");
    }
    out.push_str("</div>\n");
}

fn count_line(days: usize, decided: usize, undecided: usize) -> String {
    format!("{days} days \u{00b7} {decided} decided \u{00b7} {undecided} undecided")
}

fn month_starts(days: &[(Day, Paint)]) -> Vec<Day> {
    let mut oldest = None;
    let mut newest = None;
    for (day, _) in days {
        let first = day.first_of_month();
        oldest = Some(oldest.map_or(first, |have: Day| have.min(first)));
        newest = Some(newest.map_or(first, |have: Day| have.max(first)));
    }
    let (Some(oldest), Some(mut cursor)) = (oldest, newest) else {
        return Vec::new();
    };
    let mut months = Vec::new();
    loop {
        months.push(cursor);
        if cursor == oldest {
            break;
        }
        cursor = cursor.previous().first_of_month();
    }
    months
}

fn push_month(out: &mut String, first: Day, recorded: &BTreeMap<Day, Paint>, as_of: Day) {
    out.push_str("<section class=\"card\">\n<h2>");
    out.push_str(&month_heading(first));
    out.push_str(
        "</h2>\n<div style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr))\">",
    );
    for label in ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"] {
        out.push_str("<span>");
        out.push_str(label);
        out.push_str("</span>");
    }
    out.push_str("</div>\n<div class=\"month-grid\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr))\">");
    let days = month_days(first);
    let leading = usize::from(first.weekday());
    for _ in 0..leading {
        out.push_str("<span class=\"pad\"></span>");
    }
    for day in &days {
        push_cell(out, *day, recorded.get(day).copied(), as_of);
    }
    let trailing = (7 - (leading + days.len()) % 7) % 7;
    for _ in 0..trailing {
        out.push_str("<span class=\"pad\"></span>");
    }
    out.push_str("</div>\n</section>\n");
}

fn push_cell(out: &mut String, day: Day, paint: Option<Paint>, as_of: Day) {
    if day == as_of {
        push_open_now(out, day);
        return;
    }
    if day > as_of {
        push_plain(out, day);
        return;
    }
    match paint {
        Some(Paint::Decided(colour)) => push_decided(out, day, colour),
        Some(Paint::Undecided) => push_undecided(out, day),
        Some(Paint::NoRecord) | None => push_plain(out, day),
    }
}

fn push_open_now(out: &mut String, day: Day) {
    let (_, number) = day.month_day();
    out.push_str("<a href=\"../index.html\" aria-label=\"");
    push_escaped(out, &format!("{number} {}: open now", day.month_name()));
    out.push_str(
        "\" style=\"background:#FFFFFF;color:#141414;box-shadow:inset 0 0 0 3px #141414\"><span>",
    );
    out.push_str(&number.to_string());
    out.push_str("</span><span>open now</span></a>");
}

fn push_decided(out: &mut String, day: Day, colour: Colour) {
    let (_, number) = day.month_day();
    let [red, green, blue] = colour.srgb();
    out.push_str("<a href=\"");
    out.push_str(&day.to_string());
    out.push_str(".html\" aria-label=\"");
    push_escaped(
        out,
        &format!("{number} {}: {}", day.month_name(), colour.name()),
    );
    out.push_str("\" style=\"background:#");
    out.push_str(&format!("{red:02X}{green:02X}{blue:02X}"));
    out.push_str(";color:");
    out.push_str(contrasting_text(colour.srgb()));
    out.push_str("\"><span>");
    out.push_str(&number.to_string());
    out.push_str("</span><span>");
    push_escaped(out, colour.name());
    out.push_str("</span></a>");
}

fn push_undecided(out: &mut String, day: Day) {
    let (_, number) = day.month_day();
    out.push_str("<a href=\"");
    out.push_str(&day.to_string());
    out.push_str(".html\" aria-label=\"");
    push_escaped(out, &format!("{number} {}: undecided", day.month_name()));
    out.push_str("\" style=\"background:");
    out.push_str(HATCH);
    out.push_str(";color:#141414\"><span>");
    out.push_str(&number.to_string());
    out.push_str("</span><span>undecided</span></a>");
}

fn push_plain(out: &mut String, day: Day) {
    let (_, number) = day.month_day();
    out.push_str("<span class=\"plain\">");
    out.push_str(&number.to_string());
    out.push_str("</span>");
}

fn push_tally(out: &mut String, page: &ArchivePage<'_>) {
    let (counts, undecided) =
        days_per_colour(page.days.iter().filter_map(|(_, paint)| match paint {
            Paint::Decided(colour) => Some(Some(*colour)),
            Paint::Undecided => Some(None),
            Paint::NoRecord => None,
        }));
    let mut rows: Vec<(Option<Colour>, usize)> = Colour::ALL
        .into_iter()
        .map(|colour| (Some(colour), counts.get(&colour).copied().unwrap_or(0)))
        .collect();
    rows.push((None, undecided));
    rows.sort_by(|left, right| {
        right.1.cmp(&left.1).then_with(|| match (left.0, right.0) {
            (Some(left_colour), Some(right_colour)) => left_colour.cmp(&right_colour),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
    });
    let max = rows.iter().map(|(_, count)| *count).max().unwrap_or(0);
    out.push_str("<section class=\"card\" aria-labelledby=\"tally-h\">\n<h2 id=\"tally-h\">Days per colour</h2>\n<ol>\n");
    for (key, count) in rows {
        let name = key.map(|colour| colour.name()).unwrap_or("undecided");
        let muted = if count == 0 { " muted" } else { "" };
        out.push_str("<li data-tally=\"");
        out.push_str(name);
        out.push_str("\"><span class=\"name");
        out.push_str(muted);
        out.push_str("\">");
        out.push_str(name);
        out.push_str("</span><span class=\"count\">");
        out.push_str(&count.to_string());
        out.push_str("</span><span style=\"background:");
        out.push_str(&key.map(paint_colour).unwrap_or_else(|| HATCH.to_owned()));
        out.push_str("\"></span><span style=\"width:");
        let width = count * 100 / max.max(1);
        out.push_str(&width.to_string());
        out.push_str("%\"></span></li>\n");
    }
    out.push_str("</ol>\n");
    out.push_str("<details class=\"formula\"><summary>formula</summary><p class=\"mono\">");
    out.push_str(DAYS_PER_COLOUR_RULE);
    out.push_str("</p></details></section>\n");
}

fn month_days(first: Day) -> Vec<Day> {
    let mut days = Vec::new();
    let mut cursor = first;
    loop {
        days.push(cursor);
        let next = cursor.next();
        if next.month_day().0 != first.month_day().0 {
            break;
        }
        cursor = next;
    }
    days
}

fn month_heading(day: Day) -> String {
    format!("{} {}", day.month_name(), year_of(day))
}

fn year_of(day: Day) -> String {
    day.to_string().split('-').next().unwrap_or("").to_owned()
}

fn paint_background(paint: Paint) -> String {
    match paint {
        Paint::Decided(colour) => paint_colour(colour),
        Paint::Undecided => HATCH.to_owned(),
        Paint::NoRecord => "#F3F2EE".to_owned(),
    }
}

fn paint_colour(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!("#{red:02X}{green:02X}{blue:02X}")
}

fn contrasting_text(rgb: [u8; 3]) -> &'static str {
    let luminance = relative_luminance(rgb);
    let white = 1.05 / (luminance + 0.05);
    let black = (luminance + 0.05) / 0.05;
    if white > black { "#FFFFFF" } else { "#000000" }
}

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let linear = |channel: u8| {
        let unit = f64::from(channel) / 255.0;
        if unit <= 0.03928 {
            unit / 12.92
        } else {
            ((unit + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::render_archive;
    use crate::colour::Colour::{self, Blue};
    use crate::day::Day;
    use crate::site::support::{FORBIDDEN, assert_clean};
    use crate::site::{ArchivePage, Chrome, Paint};
    use crate::tally::{DAYS_PER_COLOUR_RULE, days_per_colour};

    #[test]
    fn render_archive_is_a_calendar_not_a_nested_days_path() {
        let days = [
            ("1970-01-01".parse().unwrap(), Paint::Decided(Blue)),
            ("1970-01-03".parse().unwrap(), Paint::Undecided),
        ];
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: chrome(0, Paint::NoRecord, &names),
        });
        assert!(html.contains("<h1>The days</h1>"));
        assert!(html.contains("href=\"../days/index.html\" aria-current=\"page\""));
        assert!(html.contains("background:#0000FF"));
        assert!(html.contains("undecided"));
        assert!(!html.contains("days/1970-01-01.html"));
        assert!(!html.contains("days/1970-01-03.html"));
        assert_clean(&html);
    }

    #[test]
    fn october_archive_paints_the_stripe_calendar_and_tally() {
        let start: Day = "2026-10-05".parse().unwrap();
        let end: Day = "2026-10-25".parse().unwrap();
        let undecided_on = ["2026-10-07", "2026-10-11", "2026-10-16", "2026-10-24"];
        let mut days = Vec::new();
        let mut cursor = start;
        loop {
            let paint = if undecided_on.contains(&cursor.to_string().as_str()) {
                Paint::Undecided
            } else {
                Paint::Decided(Blue)
            };
            days.push((cursor, paint));
            if cursor == end {
                break;
            }
            cursor = cursor.next();
        }
        assert_eq!(days.len(), 21);
        let today: Day = "2026-10-26".parse().unwrap();
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: chrome(today.open_ms(), Paint::NoRecord, &names),
        });

        let stripe = section_until(&html, "class=\"stripe\"", "<p><span>");
        assert_eq!(stripe.matches("stripe-seg").count(), 21);
        assert_eq!(stripe.matches("repeating-linear-gradient").count(), 4);
        assert!(html.contains("2026-10-05"));
        assert!(html.contains("2026-10-25"));
        assert!(html.contains("21 days \u{00b7} 17 decided \u{00b7} 4 undecided"));

        let october = section_until(&html, "<h2>October 2026</h2>", "Days per colour");
        assert!(october.starts_with("<h2>October 2026</h2>"));
        assert!(october.contains(
            "<div class=\"month-grid\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr))\"><span class=\"pad\"></span><span class=\"pad\"></span><span class=\"pad\"></span>"
        ));
        assert!(
            october.contains("<span>MON</span><span>TUE</span><span>WED</span><span>THU</span>")
        );
        for day in 1..=4 {
            assert!(
                !october.contains(&format!("href=\"2026-10-{day:02}.html\"")),
                "{day}"
            );
            assert!(october.contains(&format!("<span class=\"plain\">{day}</span>")));
        }
        assert!(october.contains("href=\"2026-10-05.html\""));
        assert!(october.contains("aria-label=\"5 October: blue\""));
        assert!(october.contains("aria-label=\"7 October: undecided\""));
        assert!(october.contains("<a href=\"../index.html\" aria-label=\"26 October: open now\""));
        assert!(october.contains(">open now</span>"));
        for day in 27..=31 {
            assert!(!october.contains(&format!("href=\"2026-10-{day}.html\"")));
        }
        assert_eq!(october.matches("<h2>").count(), 1);

        let (counts, undecided) = days_per_colour(days.iter().map(|(_, paint)| match paint {
            Paint::Decided(colour) => Some(*colour),
            Paint::Undecided => None,
            Paint::NoRecord => None,
        }));
        assert_eq!(undecided, 4);
        assert_eq!(tally_names(&html).len(), 25);
        let mut expected = vec!["blue".to_owned(), "undecided".to_owned()];
        for colour in Colour::ALL {
            if colour != Blue {
                expected.push(colour.name().to_owned());
            }
        }
        assert_eq!(tally_names(&html), expected);
        for colour in Colour::ALL {
            let count = counts.get(&colour).copied().unwrap_or(0);
            assert!(
                html.contains(&format!(
                    "<span class=\"name{}\">{name}</span><span class=\"count\">{count}</span>",
                    if count == 0 { " muted" } else { "" },
                    name = colour.name()
                )),
                "{}",
                colour.name()
            );
        }
        assert!(
            html.contains("<span class=\"name\">undecided</span><span class=\"count\">4</span>")
        );
        assert!(html.contains(DAYS_PER_COLOUR_RULE));
        assert_markup_clean(&html);
    }

    #[test]
    fn two_months_render_newest_first() {
        let days = [
            ("2026-09-30".parse().unwrap(), Paint::Undecided),
            ("2026-10-05".parse().unwrap(), Paint::Decided(Blue)),
        ];
        let today: Day = "2026-10-26".parse().unwrap();
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: chrome(today.open_ms(), Paint::NoRecord, &names),
        });
        let october = html.find("<h2>October 2026</h2>").expect("October");
        let september = html.find("<h2>September 2026</h2>").expect("September");
        assert!(october < september);
        assert!(html.contains("href=\"2026-09-30.html\""));
        assert!(html.contains("href=\"2026-10-05.html\""));
        assert_markup_clean(&html);
    }

    fn chrome<'a>(
        as_of_ms: u64,
        painted_by: Paint,
        names: &'a BTreeMap<String, String>,
    ) -> Chrome<'a> {
        Chrome {
            painted_by,
            as_of_ms,
            rendered_at_ms: if as_of_ms == 0 { 3_723_000 } else { as_of_ms },
            server: "peer-a",
            names,
        }
    }

    fn section_until<'a>(html: &'a str, start: &str, end: &str) -> &'a str {
        let at = html
            .find(start)
            .unwrap_or_else(|| panic!("missing {start}"));
        let rest = &html[at..];
        let stop = rest.find(end).unwrap_or(rest.len());
        &rest[..stop]
    }

    fn tally_names(html: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = html;
        while let Some(at) = rest.find("data-tally=\"") {
            let after = &rest[at + "data-tally=\"".len()..];
            let end = after.find('"').expect("tally quote");
            names.push(after[..end].to_owned());
            rest = &after[end..];
        }
        names
    }

    fn assert_markup_clean(html: &str) {
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"));
        assert!(!lower.contains("<link"));
        assert!(!lower.contains("src="));
        for word in FORBIDDEN {
            assert!(!lower.contains(word), "{word}");
        }
    }
}
