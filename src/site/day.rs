//! One closed day's page.

use std::collections::{BTreeMap, BTreeSet};

use crate::closing::{Decision, LIVE_RULE, RULE};
use crate::colour::Colour;
use crate::tally::{CURVE_RULE, curve};

use super::layout::{
    card, document, push_arguments, push_changes, push_current, push_fetches, push_live,
};
use super::{DayPage, Section};

/// A closed day's archive page, painted by `chrome.painted_by`.
pub fn render_day(page: &DayPage<'_>) -> String {
    let mut body = String::new();
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
    push_changes(&mut body, page.votes, &page.chrome);
    push_fetches(&mut body, page.evidence, &page.chrome);
    push_curve(&mut body, page);
    push_arguments(&mut body, page.arguments, &page.chrome);
    document(
        &page.day.to_string(),
        Some(Section::Days),
        1,
        &page.chrome,
        &body,
    )
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

fn push_curve(out: &mut String, page: &DayPage<'_>) {
    let points = curve(page.votes, page.day.open_ms(), page.day.close_ms());
    let mut body = String::from("<p>");
    body.push_str(CURVE_RULE);
    body.push_str("</p>\n");
    body.push_str(&step_svg(&points));
    push_curve_table(&mut body, &points);
    out.push_str(&card("Vote curve", &body));
}

fn push_curve_table(out: &mut String, points: &[(u64, BTreeMap<Colour, usize>)]) {
    let colours = colours_in(points);
    out.push_str("<table>\n<tr><th>Time (UTC)</th>");
    for colour in &colours {
        out.push_str("<th>");
        super::layout::push_escaped(out, colour.name());
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

fn hhmm(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) % 86_400;
    let hour = secs / 3_600;
    let minute = (secs % 3_600) / 60;
    format!("{hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::render_day;
    use crate::closing::{Decision, HOLD_MS};
    use crate::closing::{LIVE_RULE, RULE};
    use crate::colour::Colour::Blue;
    use crate::site::Paint;
    use crate::site::support::{assert_clean, body_open, page, vote};
    use crate::tally::CURVE_RULE;

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
        assert!(html.contains("<body style=\"background:#0000FF;color:#FFFFFF\">"));
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
        assert_eq!(body_open(&html), "<body>");
        assert_clean(&html);
    }
}
