//! The days: a painted stripe, a Monday-first calendar, and days per colour.

use std::collections::BTreeMap;

use crate::colour::Colour;
use crate::day::Day;
use crate::tally::{DAYS_PER_COLOUR_RULE, days_per_colour};

use super::layout::{document, push_escaped};
use super::{ArchivePage, Paint, Section};

const HATCH: &str = "repeating-linear-gradient(45deg,#E4E3DE 0 6px,#F3F2EE 6px 12px)";
const TILE_STYLE: &str = "aspect-ratio:1/1;border-radius:14px;display:flex;flex-direction:column;justify-content:space-between;padding:10px;box-sizing:border-box;min-width:0;overflow:hidden;text-decoration:none;";
const NUMBER_SPAN: &str = "<span style=\"font-weight:800;font-size:20px;line-height:1\">";
const NAME_SPAN: &str =
    "<span style=\"font-size:13px;font-weight:600;line-height:1.1;overflow-wrap:anywhere\">";

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

    let mut body = String::from(
        "<section class=\"hero\" style=\"padding-bottom:36px\"><div class=\"hero-text\"><h1>The days</h1>\n<p class=\"lede\">",
    );
    body.push_str(
        "Every day the swarm has closed, in the colour it chose. A day with no colour held by more than half for an hour stays unpainted.",
    );
    body.push_str("</p>\n");
    push_stripe(&mut body, &ordered, decided, undecided);
    body.push_str("<div class=\"mono\" style=\"display:flex;justify-content:space-between;font-size:13px\"><span>");
    if let (Some((first, _)), Some((last, _))) = (ordered.first(), ordered.last()) {
        body.push_str(&short_date(*first));
        body.push_str("</span><span>");
        body.push_str(&count_line(ordered.len(), decided, undecided));
        body.push_str("</span><span>");
        body.push_str(&short_date(*last));
    } else {
        body.push_str(&count_line(0, 0, 0));
    }
    body.push_str("</span></div>\n</div></section>\n<div class=\"split\"><div class=\"main\">\n");

    let as_of = Day::from_ms(page.chrome.as_of_ms);
    for (index, month) in month_starts(page.days, as_of).into_iter().enumerate() {
        push_month(&mut body, month, &recorded, as_of, index == 0);
    }
    body.push_str("</div>\n");
    push_tally(&mut body, page);
    body.push_str("</div>\n");
    document("The days", Some(Section::Days), 1, &page.chrome, &body)
}

fn recorded_days(page: &ArchivePage<'_>) -> BTreeMap<Day, Paint> {
    let mut recorded = BTreeMap::new();
    for (day, paint) in page.days {
        recorded.insert(*day, *paint);
    }
    recorded
}

fn push_stripe(out: &mut String, days: &[&(Day, Paint)], decided: usize, undecided: usize) {
    out.push_str("<div class=\"stripe\" role=\"img\" aria-label=\"");
    let unit = if days.len() == 1 { "day" } else { "days" };
    if let (Some((first, _)), Some((last, _))) = (days.first(), days.last()) {
        push_escaped(
            out,
            &format!(
                "Every closed day as one stripe, oldest left: {} {unit} from {} {} to {} {}, {decided} decided and {undecided} undecided.",
                days.len(),
                first.month_day().1,
                first.month_name(),
                last.month_day().1,
                last.month_name(),
            ),
        );
    } else {
        out.push_str("No day has closed yet.");
    }
    out.push_str("\" style=\"height:72px;border-radius:16px;box-sizing:border-box;padding:6px;background:var(--ground);box-shadow:0 0 0 1px rgba(0,0,0,0.12)\"><div style=\"display:flex;height:100%;border-radius:10px;overflow:hidden\">");
    for (_, paint) in days {
        out.push_str("<span class=\"stripe-seg\" style=\"flex:1;background:");
        out.push_str(&paint_background(*paint));
        out.push_str("\"></span>");
    }
    out.push_str("</div></div>\n");
}

fn count_line(days: usize, decided: usize, undecided: usize) -> String {
    let unit = if days == 1 { "day" } else { "days" };
    format!("{days} {unit} \u{00b7} {decided} decided \u{00b7} {undecided} undecided")
}

fn short_date(day: Day) -> String {
    format!("{} {}", day.month_day().1, &day.month_name()[..3]).to_ascii_uppercase()
}

fn month_starts(days: &[(Day, Paint)], as_of: Day) -> Vec<Day> {
    let mut oldest = as_of.first_of_month();
    let mut cursor = oldest;
    for (day, _) in days {
        let first = day.first_of_month();
        oldest = oldest.min(first);
        cursor = cursor.max(first);
    }
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

fn push_month(
    out: &mut String,
    first: Day,
    recorded: &BTreeMap<Day, Paint>,
    as_of: Day,
    newest: bool,
) {
    out.push_str("<section class=\"card\">\n<h2 style=\"font-size:30px\">");
    out.push_str(&month_heading(first));
    out.push_str(
        "</h2>\n<div class=\"mono muted\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:8px;font-size:12px\">",
    );
    for label in ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"] {
        out.push_str("<span>");
        out.push_str(label);
        out.push_str("</span>");
    }
    out.push_str("</div>\n<div class=\"month-grid\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:8px\">");
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
    out.push_str("</div>\n");
    if newest {
        out.push_str("<div style=\"display:flex;flex-wrap:wrap;justify-content:space-between;gap:12px;font-size:15px\"><span class=\"muted\">plumage’s first day ");
        if let Some((day, _)) = recorded.first_key_value() {
            out.push_str(&format!(
                "was {} {} {}",
                day.weekday_name(),
                day.month_day().1,
                day.month_name(),
            ));
        } else {
            out.push_str("is today");
        }
        out.push_str("</span></div>\n");
    }
    out.push_str("</section>\n");
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
    out.push_str("\" style=\"");
    out.push_str(TILE_STYLE);
    out.push_str("background:#FFFFFF;color:#141414;box-shadow:inset 0 0 0 3px #141414\">");
    out.push_str(NUMBER_SPAN);
    out.push_str(&number.to_string());
    out.push_str("</span>");
    out.push_str(NAME_SPAN);
    out.push_str("open now</span></a>");
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
    out.push_str("\" style=\"");
    out.push_str(TILE_STYLE);
    out.push_str("background:#");
    out.push_str(&format!("{red:02X}{green:02X}{blue:02X}"));
    out.push_str(";color:");
    out.push_str(contrasting_text(colour.srgb()));
    out.push_str(";box-shadow:inset 0 0 0 1px rgba(0,0,0,0.12)\">");
    out.push_str(NUMBER_SPAN);
    out.push_str(&number.to_string());
    out.push_str("</span>");
    out.push_str(NAME_SPAN);
    push_escaped(out, colour.name());
    out.push_str("</span></a>");
}

fn push_undecided(out: &mut String, day: Day) {
    let (_, number) = day.month_day();
    out.push_str("<a href=\"");
    out.push_str(&day.to_string());
    out.push_str(".html\" aria-label=\"");
    push_escaped(out, &format!("{number} {}: undecided", day.month_name()));
    out.push_str("\" style=\"");
    out.push_str(TILE_STYLE);
    out.push_str("background:");
    out.push_str(HATCH);
    out.push_str(";color:#141414;box-shadow:inset 0 0 0 1.5px #C9C8C2\">");
    out.push_str(NUMBER_SPAN);
    out.push_str(&number.to_string());
    out.push_str("</span>");
    out.push_str(NAME_SPAN);
    out.push_str("undecided</span></a>");
}

fn push_plain(out: &mut String, day: Day) {
    let (_, number) = day.month_day();
    out.push_str("<span class=\"plain muted\" style=\"aspect-ratio:1/1;padding:10px;box-sizing:border-box;font-weight:800;font-size:20px;line-height:1\">");
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
    out.push_str("<section class=\"card side\" aria-labelledby=\"tally-h\">\n<h2 id=\"tally-h\">Days per colour</h2>\n<p class=\"muted\" style=\"font-size:14px\">Closed days whose decision named the colour. All 24 are listed; most have never been chosen.</p>\n<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:6px\">\n");
    for (key, count) in rows {
        let name = key.map(|colour| colour.name()).unwrap_or("undecided");
        let muted = if count == 0 { " muted" } else { "" };
        out.push_str("<li data-tally=\"");
        out.push_str(name);
        out.push_str("\" style=\"display:grid;grid-template-columns:22px 78px minmax(0,1fr) 24px;align-items:center;gap:10px;font-size:14px\"><span aria-hidden=\"true\" style=\"width:22px;height:22px;border-radius:6px;background:");
        out.push_str(&key.map(paint_colour).unwrap_or_else(|| HATCH.to_owned()));
        out.push_str(";box-shadow:inset 0 0 0 1px rgba(0,0,0,0.15)\"></span><span class=\"name");
        out.push_str(muted);
        out.push_str("\" style=\"font-weight:600\">");
        out.push_str(name);
        out.push_str("</span><span style=\"height:10px;border-radius:5px;background:#F0F0EC;overflow:hidden;display:block\"><span style=\"display:block;height:100%;width:");
        let width = count * 100 / max.max(1);
        out.push_str(&width.to_string());
        out.push_str("%;background:#141414\"></span></span><span class=\"count mono");
        out.push_str(muted);
        out.push_str("\" style=\"text-align:right\">");
        out.push_str(&count.to_string());
        out.push_str("</span></li>\n");
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

        assert!(html.contains(
            "<section class=\"hero\" style=\"padding-bottom:36px\"><div class=\"hero-text\">"
        ));
        assert!(html.contains("<p class=\"lede\">Every day the swarm has closed"));
        assert!(html.contains("<div class=\"split\"><div class=\"main\">"));
        assert!(html.contains("</div>\n<section class=\"card side\" aria-labelledby=\"tally-h\">"));
        let stripe = section_until(&html, "class=\"stripe\"", "</div>");
        assert_eq!(stripe.matches("stripe-seg").count(), 21);
        assert_eq!(stripe.matches("repeating-linear-gradient").count(), 4);
        assert!(stripe.contains("aria-label=\"Every closed day as one stripe, oldest left: 21 days from 5 October to 25 October, 17 decided and 4 undecided.\""));
        assert!(stripe.contains("height:72px;border-radius:16px;box-sizing:border-box;padding:6px;background:var(--ground);box-shadow:0 0 0 1px rgba(0,0,0,0.12)"));
        assert!(stripe.contains(
            "<div style=\"display:flex;height:100%;border-radius:10px;overflow:hidden\">"
        ));
        assert!(!stripe.contains("background:#FFFFFF;box-shadow"));
        assert!(html.contains("<div class=\"mono\" style=\"display:flex;justify-content:space-between;font-size:13px\"><span>5 OCT</span><span>21 days · 17 decided · 4 undecided</span><span>25 OCT</span></div>"));

        let october = section_until(
            &html,
            "<h2 style=\"font-size:30px\">October 2026</h2>",
            "</section>",
        );
        assert!(october.contains("<div class=\"mono muted\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:8px;font-size:12px\">"));
        assert!(october.contains(
            "<div class=\"month-grid\" style=\"display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:8px\"><span class=\"pad\"></span><span class=\"pad\"></span><span class=\"pad\"></span>"
        ));
        assert!(
            october.contains("<span>MON</span><span>TUE</span><span>WED</span><span>THU</span>")
        );
        for day in 1..=4 {
            assert!(
                !october.contains(&format!("href=\"2026-10-{day:02}.html\"")),
                "{day}"
            );
            assert!(october.contains(&format!("<span class=\"plain muted\" style=\"aspect-ratio:1/1;padding:10px;box-sizing:border-box;font-weight:800;font-size:20px;line-height:1\">{day}</span>")));
        }
        assert!(october.contains("href=\"2026-10-05.html\""));
        assert!(october.contains("aria-label=\"5 October: blue\""));
        assert!(october.contains("aria-label=\"7 October: undecided\""));
        assert!(october.contains("<a href=\"../index.html\" aria-label=\"26 October: open now\""));
        assert!(october.contains(">open now</span>"));
        for (label, number, name, paint) in [
            (
                "5 October: blue",
                5,
                "blue",
                "background:#0000FF;color:#FFFFFF",
            ),
            (
                "7 October: undecided",
                7,
                "undecided",
                "background:repeating-linear-gradient",
            ),
            (
                "26 October: open now",
                26,
                "open now",
                "box-shadow:inset 0 0 0 3px #141414",
            ),
        ] {
            let tile = section_until(october, &format!("aria-label=\"{label}\""), "</a>");
            assert!(tile.contains("aspect-ratio:1/1;border-radius:14px;display:flex;flex-direction:column;justify-content:space-between;padding:10px;box-sizing:border-box;min-width:0;overflow:hidden;text-decoration:none;"));
            assert!(tile.contains(&format!(
                "<span style=\"font-weight:800;font-size:20px;line-height:1\">{number}</span>"
            )));
            assert!(tile.contains(&format!("<span style=\"font-size:13px;font-weight:600;line-height:1.1;overflow-wrap:anywhere\">{name}</span>")));
            assert!(tile.contains(paint));
        }
        for day in 27..=31 {
            assert!(!october.contains(&format!("href=\"2026-10-{day}.html\"")));
        }
        assert_eq!(html.matches("class=\"month-grid\"").count(), 1);
        assert!(october.contains("plumage’s first day was Monday 5 October"));

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
            let row = section_until(
                &html,
                &format!("<li data-tally=\"{}\"", colour.name()),
                "</li>",
            );
            let muted = if count == 0 { " muted" } else { "" };
            assert!(row.contains("display:grid;grid-template-columns:22px 78px minmax(0,1fr) 24px;align-items:center;gap:10px;font-size:14px"));
            assert!(row.contains(&format!(
                "<span class=\"name{muted}\" style=\"font-weight:600\">{}</span>",
                colour.name()
            )));
            assert!(row.contains(&format!(
                "<span class=\"count mono{muted}\" style=\"text-align:right\">{count}</span>"
            )));
            assert!(row.contains(&format!(
                "display:block;height:100%;width:{}%;background:#141414",
                count * 100 / 17
            )));
            assert!(row.contains("width:22px;height:22px;border-radius:6px;background:"));
        }
        let row = section_until(&html, "<li data-tally=\"undecided\"", "</li>");
        assert!(row.contains("<span class=\"name\" style=\"font-weight:600\">undecided</span>"));
        assert!(row.contains("<span class=\"count mono\" style=\"text-align:right\">4</span>"));
        assert!(html.contains(DAYS_PER_COLOUR_RULE));
        assert_markup_clean(&html);
    }

    #[test]
    fn one_day_archive_uses_singular_in_the_count_and_stripe_label() {
        let day: Day = "2026-10-30".parse().unwrap();
        let days = [(day, Paint::Decided(Blue))];
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: chrome(day.next().open_ms(), Paint::Decided(Blue), &names),
        });
        assert!(html.contains("1 day · 1 decided · 0 undecided"));
        assert!(html.contains("aria-label=\"Every closed day as one stripe, oldest left: 1 day from 30 October to 30 October, 1 decided and 0 undecided.\""));
        assert_eq!(html.matches("stripe-seg").count(), 1);
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
        let october = html.find(">October 2026</h2>").expect("October");
        let september = html.find(">September 2026</h2>").expect("September");
        assert!(october < september);
        assert!(html.contains("href=\"2026-09-30.html\""));
        assert!(html.contains("href=\"2026-10-05.html\""));
        let first_day = html
            .find("plumage’s first day was Wednesday 30 September")
            .expect("first day");
        assert!(october < first_day && first_day < september);
        assert_eq!(html.matches("plumage’s first day").count(), 1);
        assert_markup_clean(&html);
    }

    #[test]
    fn empty_archive_draws_the_current_month_and_zero_tally() {
        let today: Day = "2026-10-30".parse().unwrap();
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &[],
            chrome: chrome(today.open_ms(), Paint::NoRecord, &names),
        });
        assert!(html.contains(">October 2026</h2>"));
        assert_eq!(html.matches("class=\"month-grid\"").count(), 1);
        assert!(html.contains("aria-label=\"30 October: open now\""));
        assert!(html.contains("plumage’s first day is today"));
        assert!(html.contains("aria-label=\"No day has closed yet.\""));
        assert!(!html.contains("stripe-seg"));
        assert!(html.contains("<span>0 days · 0 decided · 0 undecided</span></div>"));
        assert_eq!(tally_names(&html).len(), 25);
        assert_eq!(
            html.matches("class=\"count mono muted\" style=\"text-align:right\">0</span>")
                .count(),
            25
        );
        assert_eq!(html.matches("width:0%;background:#141414").count(), 25);
        assert_markup_clean(&html);
    }

    #[test]
    fn current_month_is_drawn_even_without_a_record_in_it() {
        let days = [("2026-09-30".parse().unwrap(), Paint::Undecided)];
        let today: Day = "2026-11-01".parse().unwrap();
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: chrome(today.open_ms(), Paint::NoRecord, &names),
        });
        let november = html.find(">November 2026</h2>").expect("November");
        let october = html.find(">October 2026</h2>").expect("October");
        let september = html.find(">September 2026</h2>").expect("September");
        assert!(november < october && october < september);
        assert_eq!(html.matches("class=\"month-grid\"").count(), 3);
        assert!(html.contains("aria-label=\"1 November: open now\""));
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
