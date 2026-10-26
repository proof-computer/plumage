//! Self-contained HTML for one day and for the archive index.
//!
//! Pure functions of the records the caller passes. No file, network, store
//! or clock. `rendered_at_ms` is the caller's clock.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::closing::{Decision, LIVE_RULE, RULE, Vote, live_voters};
use crate::colour::Colour;
use crate::day::{Day, utc_time};
use crate::evidence::Evidence;
use crate::tally::{
    COUNTS_RULE, CURRENT_VOTES_RULE, CURVE_RULE, FETCHES_RULE, VOTE_CHANGES_RULE, counts,
    current_votes, curve, fetches, vote_changes,
};

/// One argument, already resolved to the evidence records it cites.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentView {
    pub id: String,
    pub author: String,
    pub at_ms: u64,
    pub text: String,
    pub cites: Vec<Evidence>,
}

/// How a page is painted. `Undecided` and `NoRecord` set no background colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paint {
    Decided(Colour),
    Undecided,
    NoRecord,
}

/// The records and clock readings for one day's pages.
#[derive(Debug)]
pub struct DayPage<'a> {
    pub day: Day,
    pub votes: &'a [Vote],
    pub arguments: &'a [ArgumentView],
    pub evidence: &'a [Evidence],
    pub decision: Option<Decision>,
    pub painted_by: Paint,
    pub as_of_ms: u64,
    pub rendered_at_ms: u64,
}

const STYLESHEET: &str = "\
body { font-family: sans-serif; line-height: 1.4; max-width: 40rem; }
table { border-collapse: collapse; }
td, th { border: 1px solid; padding: 0.2rem; vertical-align: top; }
";

/// Today's page, painted by `painted_by` (yesterday's result).
pub fn render_front(page: &DayPage<'_>) -> String {
    let mut body = String::new();
    push_freshness(&mut body, page.as_of_ms, page.rendered_at_ms);
    body.push_str("<h1>Today ");
    body.push_str(&page.day.to_string());
    body.push_str("</h1>\n<p>");
    body.push_str(&yesterday(page.painted_by));
    body.push_str("</p>\n");
    push_current(&mut body, page);
    push_live(&mut body, page);
    push_changes(&mut body, page.votes);
    push_fetches(&mut body, page.evidence);
    push_arguments(&mut body, page.arguments);
    document(&format!("Today {}", page.day), page.painted_by, &body)
}

/// A closed day's archive page, painted by `painted_by`.
pub fn render_day(page: &DayPage<'_>) -> String {
    let mut body = String::new();
    push_freshness(&mut body, page.as_of_ms, page.rendered_at_ms);
    body.push_str("<h1>");
    body.push_str(&page.day.to_string());
    body.push_str("</h1>\n");
    if let Some(decision) = page.decision {
        body.push_str("<p>");
        body.push_str(&decision_line(decision));
        body.push_str("</p>\n");
    }
    body.push_str("<p>");
    body.push_str(RULE);
    body.push_str("</p>\n<p>");
    body.push_str(LIVE_RULE);
    body.push_str("</p>\n");
    push_current(&mut body, page);
    push_live(&mut body, page);
    push_changes(&mut body, page.votes);
    push_fetches(&mut body, page.evidence);
    push_curve(&mut body, page);
    push_arguments(&mut body, page.arguments);
    document(&page.day.to_string(), page.painted_by, &body)
}

/// The index of days, newest first. Each entry links `days/<YYYY-MM-DD>.html`.
pub fn render_archive(days: &[(Day, Paint)]) -> String {
    let mut ordered: Vec<&(Day, Paint)> = days.iter().collect();
    ordered.sort_by_key(|day| Reverse(day.0));
    let mut body = String::from("<h1>Archive</h1>\n<ul>\n");
    for (day, paint) in ordered {
        body.push_str("<li><a href=\"days/");
        body.push_str(&day.to_string());
        body.push_str(".html\">");
        body.push_str(&day.to_string());
        body.push(' ');
        match paint {
            Paint::Decided(colour) => {
                push_escaped(&mut body, colour.name());
                body.push(' ');
                body.push_str(&swatch(*colour));
            }
            Paint::Undecided | Paint::NoRecord => body.push_str("undecided"),
        }
        body.push_str("</a></li>\n");
    }
    body.push_str("</ul>\n");
    document("Archive", Paint::NoRecord, &body)
}

fn document(title: &str, paint: Paint, body: &str) -> String {
    let mut html = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>",
    );
    push_escaped(&mut html, title);
    html.push_str("</title>\n<style>\n");
    html.push_str(STYLESHEET);
    html.push_str("</style>\n</head>\n");
    match paint_style(paint) {
        Some(style) => {
            html.push_str("<body style=\"");
            html.push_str(&style);
            html.push_str("\">\n");
        }
        None => html.push_str("<body>\n"),
    }
    html.push_str(body);
    html.push_str("</body>\n</html>\n");
    html
}

fn yesterday(paint: Paint) -> String {
    match paint {
        Paint::Decided(colour) => format!("Painted by yesterday: {}.", colour.name()),
        Paint::Undecided => "Yesterday was undecided under the rule".to_owned(),
        Paint::NoRecord => "No result is recorded for yesterday".to_owned(),
    }
}

fn decision_line(decision: Decision) -> String {
    match decision {
        Decision::Decided {
            colour,
            held_from_ms,
            held_to_ms,
        } => format!(
            "Decided: {}, held {}{}{} UTC",
            colour.name(),
            hhmm(held_from_ms),
            '\u{2013}',
            hhmm(held_to_ms)
        ),
        Decision::Undecided => "Undecided".to_owned(),
    }
}

/// Black or white, whichever has the higher WCAG contrast ratio against `rgb`.
fn contrasting_text(rgb: [u8; 3]) -> &'static str {
    let luminance = relative_luminance(rgb);
    let white = (1.0 + 0.05) / (luminance + 0.05);
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

fn paint_style(paint: Paint) -> Option<String> {
    match paint {
        Paint::Decided(colour) => {
            let [red, green, blue] = colour.srgb();
            Some(format!(
                "background:#{red:02X}{green:02X}{blue:02X};color:{}",
                contrasting_text(colour.srgb())
            ))
        }
        Paint::Undecided | Paint::NoRecord => None,
    }
}

fn swatch(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!("<span style=\"background:#{red:02X}{green:02X}{blue:02X};padding:0 0.5rem\"> </span>")
}

fn push_freshness(out: &mut String, as_of_ms: u64, rendered_at_ms: u64) {
    out.push_str("<p>");
    out.push_str(&format!(
        "Records as of {} UTC on {}; rendered {} {} UTC",
        utc_time(as_of_ms),
        Day::from_ms(as_of_ms),
        Day::from_ms(rendered_at_ms),
        utc_time(rendered_at_ms)
    ));
    out.push_str("</p>\n");
}

fn push_current(out: &mut String, page: &DayPage<'_>) {
    let current = current_votes(page.votes, page.as_of_ms);
    let counts = counts(page.votes, page.as_of_ms);
    let mut grouped: BTreeMap<Colour, Vec<&str>> = BTreeMap::new();
    for (author, colour) in &current {
        grouped.entry(*colour).or_default().push(author);
    }
    out.push_str("<section>\n<h2>Current votes</h2>\n<p>");
    out.push_str(CURRENT_VOTES_RULE);
    out.push_str("</p>\n<p>");
    out.push_str(COUNTS_RULE);
    out.push_str("</p>\n");
    for (colour, authors) in grouped {
        out.push_str("<h3>");
        push_escaped(out, colour.name());
        out.push_str("</h3>\n<p>");
        out.push_str(&counts.get(&colour).copied().unwrap_or(0).to_string());
        out.push_str("</p>\n<ul>\n");
        for author in authors {
            out.push_str("<li>");
            push_escaped(out, author);
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
}

fn push_live(out: &mut String, page: &DayPage<'_>) {
    let live = live_voters(page.votes, page.day.open_ms(), page.day.close_ms());
    out.push_str("<section>\n<h2>Live voters</h2>\n<p>");
    out.push_str(LIVE_RULE);
    out.push_str("</p>\n<p>Threshold: &gt; ");
    out.push_str(&live.len().to_string());
    out.push_str(" / 2</p>\n<ul>\n");
    for author in &live {
        out.push_str("<li>");
        push_escaped(out, author);
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n</section>\n");
}

fn push_changes(out: &mut String, votes: &[Vote]) {
    push_author_counts(out, "Vote changes", VOTE_CHANGES_RULE, &vote_changes(votes));
}

fn push_fetches(out: &mut String, evidence: &[Evidence]) {
    push_author_counts(out, "Fetches", FETCHES_RULE, &fetches(evidence));
}

fn push_author_counts(
    out: &mut String,
    heading: &str,
    rule: &str,
    counts: &BTreeMap<String, usize>,
) {
    out.push_str("<section>\n<h2>");
    push_escaped(out, heading);
    out.push_str("</h2>\n<p>");
    out.push_str(rule);
    out.push_str("</p>\n<ul>\n");
    for (author, count) in counts {
        out.push_str("<li>");
        push_escaped(out, author);
        out.push_str(": ");
        out.push_str(&count.to_string());
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n</section>\n");
}

fn push_curve(out: &mut String, page: &DayPage<'_>) {
    let points = curve(page.votes, page.day.open_ms(), page.day.close_ms());
    out.push_str("<section>\n<h2>Vote curve</h2>\n<p>");
    out.push_str(CURVE_RULE);
    out.push_str("</p>\n");
    out.push_str(&step_svg(&points));
    push_curve_table(out, &points);
    out.push_str("</section>\n");
}

fn push_curve_table(out: &mut String, points: &[(u64, BTreeMap<Colour, usize>)]) {
    let colours = colours_in(points);
    out.push_str("<table>\n<tr><th>Time (UTC)</th>");
    for colour in &colours {
        out.push_str("<th>");
        push_escaped(out, colour.name());
        out.push_str("</th>");
    }
    out.push_str("</tr>\n");
    for (at_ms, counts) in points {
        out.push_str("<tr><td>");
        out.push_str(&hhmm(*at_ms));
        out.push_str("</td>");
        for colour in &colours {
            out.push_str("<td>");
            out.push_str(&counts.get(colour).copied().unwrap_or(0).to_string());
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

fn step_svg(points: &[(u64, BTreeMap<Colour, usize>)]) -> String {
    let colours = colours_in(points);
    if points.is_empty() || colours.is_empty() {
        return String::new();
    }
    let max_count = points
        .iter()
        .flat_map(|(_, counts)| counts.values().copied())
        .max()
        .unwrap_or(1)
        .max(1);
    let height = 100usize;
    let width = points.len().saturating_mul(10);
    let mut svg = format!("<svg viewBox=\"0 0 {width} {height}\">\n");
    for colour in colours {
        let [red, green, blue] = colour.srgb();
        let mut drawn = String::new();
        let mut previous_y = 0usize;
        for (index, (_, counts)) in points.iter().enumerate() {
            let count = counts.get(&colour).copied().unwrap_or(0);
            let x = index.saturating_mul(10);
            let y = y_of(count, max_count, height);
            if index == 0 {
                drawn.push_str(&format!("{x},{y}"));
            } else {
                drawn.push_str(&format!(" {x},{previous_y} {x},{y}"));
            }
            previous_y = y;
        }
        svg.push_str(&format!(
            "<polyline fill=\"none\" stroke=\"#{red:02X}{green:02X}{blue:02X}\" points=\"{drawn}\"/>\n"
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

fn y_of(count: usize, max_count: usize, height: usize) -> usize {
    let scaled = count.saturating_mul(height) / max_count.max(1);
    height.saturating_sub(scaled.min(height))
}

fn colours_in(points: &[(u64, BTreeMap<Colour, usize>)]) -> BTreeSet<Colour> {
    let mut colours = BTreeSet::new();
    for (_, counts) in points {
        colours.extend(counts.keys().copied());
    }
    colours
}

fn push_arguments(out: &mut String, arguments: &[ArgumentView]) {
    out.push_str("<section>\n<h2>Arguments</h2>\n");
    let mut ordered: Vec<&ArgumentView> = arguments.iter().collect();
    ordered.sort_by(|left, right| {
        (left.at_ms, left.author.as_str(), left.id.as_str()).cmp(&(
            right.at_ms,
            right.author.as_str(),
            right.id.as_str(),
        ))
    });
    for argument in ordered {
        out.push_str("<article>\n<p>");
        out.push_str(&format!(
            "{} {} UTC",
            Day::from_ms(argument.at_ms),
            utc_time(argument.at_ms)
        ));
        out.push_str("</p>\n<p>");
        push_escaped(out, &argument.author);
        out.push_str("</p>\n<p>");
        push_escaped(out, &argument.id);
        out.push_str("</p>\n<p>");
        push_escaped(out, &argument.text);
        out.push_str("</p>\n");
        if argument.cites.is_empty() {
            out.push_str("<p>cites no evidence</p>\n");
        } else {
            out.push_str("<ul>\n");
            for cite in &argument.cites {
                out.push_str("<li>");
                push_escaped(out, cite.url().as_str());
                out.push(' ');
                push_escaped(out, cite.excerpt());
                out.push_str("</li>\n");
            }
            out.push_str("</ul>\n");
        }
        out.push_str("</article>\n");
    }
    out.push_str("</section>\n");
}

fn hhmm(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) % 86_400;
    let hour = secs / 3_600;
    let minute = (secs % 3_600) / 60;
    format!("{hour:02}:{minute:02}")
}

fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;
    use crate::closing::HOLD_MS;
    use crate::colour::Colour::{Black, Blue, Yellow};

    const RAW: &str = "</p><script>alert(1)</script> & \"x\"";
    const ESCAPED: &str = "&lt;/p&gt;&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;x&quot;";
    const FORBIDDEN: &[&str] = &[
        "faction",
        "persuasion",
        "persuaded",
        "convinced",
        "camp",
        "coalition",
        "winner",
    ];

    fn vote(author: &str, colour: Colour, at_ms: u64) -> Vote {
        Vote {
            author: author.to_owned(),
            colour,
            at_ms,
        }
    }

    fn evidence(excerpt: &str) -> Evidence {
        Evidence::new(
            "ada".to_owned(),
            "run".to_owned(),
            Url::parse("https://example.com/fact").unwrap(),
            0,
            excerpt,
        )
    }

    fn page<'a>(
        votes: &'a [Vote],
        arguments: &'a [ArgumentView],
        evidence: &'a [Evidence],
        decision: Option<Decision>,
        painted_by: Paint,
    ) -> DayPage<'a> {
        DayPage {
            day: Day::from_ms(0),
            votes,
            arguments,
            evidence,
            decision,
            painted_by,
            as_of_ms: 0,
            rendered_at_ms: 3_723_000,
        }
    }

    fn assert_clean(html: &str) {
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"), "{html}");
        assert!(!lower.contains("<link"));
        assert!(!lower.contains("src="));
        for word in FORBIDDEN {
            assert!(!lower.contains(word), "{word} in {html}");
        }
        assert!(html.contains(
            "Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC"
        ));
    }

    #[test]
    fn render_day_paints_a_blue_decision_and_publishes_both_rules() {
        let votes = [vote("ada", Blue, 0), vote("bea", Blue, 0)];
        let shown = page(
            &votes,
            &[],
            &[],
            Some(Decision::Decided {
                colour: Blue,
                held_from_ms: 0,
                held_to_ms: HOLD_MS,
            }),
            Paint::Decided(Blue),
        );
        let html = render_day(&shown);
        assert!(html.contains("background:#0000FF"));
        assert!(html.contains("blue"));
        assert!(html.contains("held 00:00\u{2013}01:00 UTC"));
        assert!(html.contains(RULE));
        assert!(html.contains(LIVE_RULE));
        assert!(html.contains(CURVE_RULE));
        assert!(html.contains("stroke=\"#0000FF\""));
        assert_clean(&html);
    }

    #[test]
    fn render_day_undecided_sets_no_background() {
        let shown = page(&[], &[], &[], Some(Decision::Undecided), Paint::Undecided);
        let html = render_day(&shown);
        assert!(html.contains("Undecided"));
        assert!(!html.contains("background:#"));
        assert_clean(&html);
    }

    #[test]
    fn render_front_paints_black_with_white_text_and_yellow_with_black_text() {
        let black = page(&[], &[], &[], None, Paint::Decided(Black));
        let html = render_front(&black);
        assert!(html.contains("background:#000000"));
        assert!(html.contains("color:#FFFFFF"));
        assert_clean(&html);

        let yellow = page(&[], &[], &[], None, Paint::Decided(Yellow));
        let html = render_front(&yellow);
        assert!(html.contains("background:#FFFF00"));
        assert!(html.contains("color:#000000"));
        assert!(!html.contains("color:#FFFFFF"));
    }

    #[test]
    fn render_front_undecided_and_no_record_paint_nothing() {
        let undecided = page(&[], &[], &[], None, Paint::Undecided);
        let html = render_front(&undecided);
        assert!(html.contains("Yesterday was undecided under the rule"));
        assert!(!html.contains("background:#"));
        assert_clean(&html);

        let missing = page(&[], &[], &[], None, Paint::NoRecord);
        let html = render_front(&missing);
        assert!(html.contains("No result is recorded for yesterday"));
        assert!(!html.contains("background:#"));
    }

    #[test]
    fn argument_text_and_excerpt_are_escaped_and_empty_cites_are_named() {
        let arguments = [
            ArgumentView {
                id: "arg-2".to_owned(),
                author: "bea".to_owned(),
                at_ms: 1,
                text: "plain".to_owned(),
                cites: Vec::new(),
            },
            ArgumentView {
                id: "arg-1".to_owned(),
                author: "ada".to_owned(),
                at_ms: 0,
                text: RAW.to_owned(),
                cites: vec![evidence(RAW)],
            },
        ];
        let shown = page(&[], &arguments, &[], None, Paint::NoRecord);
        for html in [render_front(&shown), render_day(&shown)] {
            assert!(html.contains(ESCAPED));
            assert!(!html.contains(RAW));
            assert!(!html.contains("<script"));
            assert!(html.contains("cites no evidence"));
            assert!(html.contains("https://example.com/fact"));
            let raw_at = html.find("arg-1").expect("earlier argument id");
            let plain_at = html.find("arg-2").expect("later argument id");
            assert!(raw_at < plain_at);
        }
    }

    #[test]
    fn render_archive_lists_days_newest_first() {
        let days = [
            ("2025-10-26".parse().unwrap(), Paint::Decided(Blue)),
            ("2025-10-27".parse().unwrap(), Paint::Undecided),
            ("2025-10-25".parse().unwrap(), Paint::NoRecord),
        ];
        let html = render_archive(&days);
        let newest = html.find("days/2025-10-27.html").unwrap();
        let middle = html.find("days/2025-10-26.html").unwrap();
        let oldest = html.find("days/2025-10-25.html").unwrap();
        assert!(newest < middle && middle < oldest);
        assert!(html.contains("blue"));
        assert!(html.contains("background:#0000FF"));
        assert!(html.contains("undecided"));
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"));
        assert!(!lower.contains("<link"));
        assert!(!lower.contains("src="));
        for word in FORBIDDEN {
            assert!(!lower.contains(word), "{word}");
        }
    }
}
