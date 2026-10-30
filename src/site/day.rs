//! One day's page: the decision, the stacked hourly curve, and the thread.

use std::collections::BTreeMap;

use crate::closing::{Decision, HOLD_MS, LIVE_RULE, RULE, holds, live_voters};
use crate::colour::Colour;
use crate::day::Day;
use crate::evidence::{Evidence, SUNRISE_HOST, WIKIMEDIA_HOST};
use crate::tally::{
    ARGUMENTS_RULE, CURVE_RULE, FETCHES_RULE, VOTE_CHANGES_RULE, VOTES_CAST_RULE, citations,
    current_votes, curve, fetches, trails, vote_changes, votes_cast,
};

use super::layout::{chip, document, peer, push_escaped};
use super::{DayPage, Paint, Section};

const NOT_YET: &str = "repeating-linear-gradient(45deg,#F3F2EE 0 6px,#FFFFFF 6px 12px)";
const HATCH: &str = "repeating-linear-gradient(45deg,#E4E3DE 0 6px,#F3F2EE 6px 12px)";

/// A day's page at depth 1, painted by `chrome.painted_by`.
///
/// `decision: None` is the live day. A `Some` decision is that day after close.
pub fn render_day(page: &DayPage<'_>) -> String {
    let live = live_voters(page.votes, page.day.open_ms(), page.day.close_ms());
    let open = page.decision.is_none();
    let (headline, prose, rule_heading) = decision_copy(page, live.len());
    let mut body = String::new();
    push_nav(&mut body, page);
    body.push_str("<section class=\"hero\"><div class=\"hero-text\"><p class=\"mono eyebrow\">");
    push_escaped(&mut body, &status_line(page.day, open));
    body.push_str("</p>\n<h1 class=\"verdict\">");
    push_escaped(&mut body, &headline);
    body.push_str("</h1>\n<p class=\"lede\">");
    push_escaped(&mut body, &prose);
    body.push_str("</p>\n</div><section class=\"card\" style=\"flex:1 1 340px;margin:0;border-radius:20px;padding:18px 20px\"><strong>");
    push_escaped(&mut body, &rule_heading);
    body.push_str("</strong><p class=\"mono\" style=\"font-size:12.5px\">");
    body.push_str(RULE);
    body.push_str("</p>\n<p class=\"mono\" style=\"font-size:12.5px\">");
    body.push_str(LIVE_RULE);
    body.push_str("</p>\n</section></section>\n");
    push_curve(&mut body, page, live.len());
    push_numbers(&mut body, page, live.len());
    push_thread(&mut body, page, &live);
    push_facts(&mut body, page);
    document(
        &long_date(page.day),
        Some(Section::Days),
        1,
        &page.chrome,
        &body,
    )
}

fn decision_copy(page: &DayPage<'_>, live: usize) -> (String, String, String) {
    let painted = prose_date(page.day.next());
    match page.decision {
        Some(Decision::Decided {
            colour,
            held_from_ms,
            held_to_ms,
        }) => (
            format!("{}.", capitalise(colour.name())),
            format!(
                "Decided: {}, held by more than half of the {live} live voters from {} to {} UTC. It painted {painted}.",
                colour.name(),
                hhmm(held_from_ms),
                hhmm(held_to_ms)
            ),
            "The rule that decided it".to_owned(),
        ),
        Some(Decision::Undecided) => (
            "Undecided.".to_owned(),
            format!(
                "No colour held more than half of the live voters for a continuous hour before 23:00 UTC. {painted} was left unpainted."
            ),
            "The rule that decided it".to_owned(),
        ),
        None => (
            "Open.".to_owned(),
            "The question is open until 23:00 UTC.".to_owned(),
            "The rule that will decide it".to_owned(),
        ),
    }
}

fn push_nav(out: &mut String, page: &DayPage<'_>) {
    let today = Day::from_ms(page.chrome.as_of_ms);
    let next = page.next.filter(|_| page.day != today);
    if page.previous.is_none() && next.is_none() {
        return;
    }
    out.push_str("<nav aria-label=\"Days\">");
    if let Some((day, paint)) = page.previous {
        out.push_str(&format!(
            "<a href=\"{day}.html\">← {} · {}</a>",
            prose_date(day),
            outcome(paint)
        ));
    }
    if let Some((day, paint)) = next {
        if day == today {
            out.push_str(&format!(
                "<a href=\"../index.html\">{} · open now →</a>",
                prose_date(day)
            ));
        } else {
            out.push_str(&format!(
                "<a href=\"{day}.html\">{} · {} →</a>",
                prose_date(day),
                outcome(paint)
            ));
        }
    }
    out.push_str("</nav>\n");
}

fn outcome(paint: Paint) -> &'static str {
    match paint {
        Paint::Decided(colour) => colour.name(),
        Paint::Undecided => "undecided",
        Paint::NoRecord => "no record",
    }
}

fn push_curve(out: &mut String, page: &DayPage<'_>, live: usize) {
    let points = curve(page.votes, page.day.open_ms(), page.day.close_ms());
    let colours = colours_present(&points);
    let until = as_of(page);
    let has_future = page.decision.is_none() && points.iter().any(|(at_ms, _)| *at_ms > until);
    out.push_str("<section class=\"card\" aria-labelledby=\"curve-h\">\n<h2 id=\"curve-h\">How the votes moved</h2>\n<div class=\"legend\">");
    for colour in &colours {
        out.push_str("<span><span class=\"swatch-xs\" style=\"background:");
        out.push_str(&hex(*colour));
        out.push_str("\"></span>");
        push_escaped(out, colour.name());
        out.push_str("</span>");
    }
    out.push_str("<span><span class=\"swatch-xs\" style=\"background:");
    out.push_str(HATCH);
    out.push_str("\"></span>not voted yet</span>");
    if has_future {
        out.push_str("<span><span class=\"swatch-xs\" style=\"background:");
        out.push_str(NOT_YET);
        out.push_str("\"></span>not yet</span>");
    }
    out.push_str("</div>\n");
    out.push_str("<div role=\"img\" aria-label=\"");
    push_escaped(out, &curve_label(page));
    out.push_str("\" style=\"position:relative\">\n<div style=\"position:relative;height:280px;display:flex;align-items:flex-end;gap:4px;border-bottom:1.5px solid #141414\">");
    out.push_str("<div style=\"position:absolute;left:0;right:0;bottom:50%;border-top:1.5px dashed #141414\"></div>");
    out.push_str("<span class=\"mono\" style=\"position:absolute;right:0;bottom:50%;transform:translateY(-4px);font-size:11px;background:#FFFFFF;padding:0 4px\">half of ");
    out.push_str(&live.to_string());
    out.push_str("</span>");
    push_hold_outlines(out, page, &points);
    for (at_ms, counts) in &points {
        push_column(
            out,
            *at_ms,
            counts,
            live,
            page.decision.is_none() && *at_ms > until,
        );
    }
    out.push_str(
        "</div>\n<div class=\"mono muted\" style=\"display:flex;gap:4px;font-size:11px\">",
    );
    for (at_ms, _) in &points {
        out.push_str("<span style=\"flex:1;text-align:center\">");
        out.push_str(&hour_label(*at_ms));
        out.push_str("</span>");
    }
    out.push_str("</div>\n");
    push_hold_labels(out, page);
    out.push_str("</div>\n<details><summary>The values, and how they are taken</summary>\n<p class=\"mono\">");
    out.push_str(CURVE_RULE);
    out.push_str("</p>\n<table>\n<tr><th>UTC</th>");
    for colour in &colours {
        out.push_str("<th>");
        push_escaped(out, colour.name());
        out.push_str("</th>");
    }
    out.push_str("</tr>\n");
    for (at_ms, counts) in &points {
        out.push_str("<tr><td>");
        out.push_str(&hhmm(*at_ms));
        out.push_str("</td>");
        for colour in &colours {
            out.push_str("<td>");
            if page.decision.is_none() && *at_ms > until {
                out.push('–');
            } else {
                out.push_str(&counts.get(colour).copied().unwrap_or(0).to_string());
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n</details>\n</section>\n");
}

fn push_column(
    out: &mut String,
    at_ms: u64,
    counts: &BTreeMap<Colour, usize>,
    live: usize,
    future: bool,
) {
    out.push_str("<div data-hour=\"");
    out.push_str(&hhmm(at_ms));
    out.push_str("\" style=\"flex:1;height:100%;display:flex;flex-direction:column;justify-content:flex-end\"><div style=\"height:100%;display:flex;flex-direction:column-reverse\">");
    if future {
        out.push_str("<div data-colour=\"not-yet\" style=\"height:100%;background:");
        out.push_str(NOT_YET);
        out.push_str("\"></div></div></div>");
        return;
    }
    let mut named = 0usize;
    for colour in Colour::ALL {
        let count = counts.get(&colour).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        named += count;
        out.push_str("<div data-colour=\"");
        out.push_str(colour.name());
        out.push_str("\" style=\"height:");
        out.push_str(&percent(count, live));
        out.push_str("%;background:");
        out.push_str(&hex(colour));
        out.push_str("\"></div>");
    }
    let remainder = live.saturating_sub(named);
    if remainder > 0 {
        out.push_str("<div data-colour=\"not-voted-yet\" style=\"height:");
        out.push_str(&percent(remainder, live));
        out.push_str("%;background:");
        out.push_str(HATCH);
        out.push_str("\"></div>");
    }
    out.push_str("</div></div>");
}

fn push_hold_outlines(
    out: &mut String,
    page: &DayPage<'_>,
    points: &[(u64, BTreeMap<Colour, usize>)],
) {
    if points.is_empty() {
        return;
    }
    for (from_ms, to_ms) in lasting_holds(page) {
        let complete = to_ms - from_ms >= HOLD_MS;
        let width = (to_ms - from_ms) as f64 / HOLD_MS as f64 * 100.0 / points.len() as f64;
        out.push_str("<div style=\"position:absolute;top:-6px;bottom:-6px;left:");
        out.push_str(&format!("{:.2}", chart_left(from_ms, points.len())));
        out.push_str("%;width:");
        out.push_str(&format!("{width:.2}"));
        if complete {
            out.push_str("%;box-shadow:0 0 0 2.5px #141414\"></div>");
        } else {
            out.push_str("%;outline:2.5px dashed #141414\"></div>");
        }
    }
}

fn push_hold_labels(out: &mut String, page: &DayPage<'_>) {
    for (from_ms, to_ms) in lasting_holds(page) {
        if to_ms - from_ms >= HOLD_MS {
            out.push_str("<p>held ");
            out.push_str(&hhmm(from_ms));
            out.push('\u{2013}');
            out.push_str(&hhmm(to_ms));
        } else {
            out.push_str("<p>holding since ");
            out.push_str(&hhmm(from_ms));
        }
        out.push_str("</p>\n");
    }
}

fn lasting_holds(page: &DayPage<'_>) -> Vec<(u64, u64)> {
    let live = live_voters(page.votes, page.day.open_ms(), page.day.close_ms());
    let until = if page.decision.is_none() {
        as_of(page)
    } else {
        page.day.close_ms()
    };
    holds(page.votes, &live, page.day.open_ms(), until)
        .into_iter()
        .filter_map(|hold| {
            let end = hold.to_ms.unwrap_or(until);
            let qualified = hold.from_ms.checked_add(HOLD_MS)?;
            if end.saturating_sub(hold.from_ms) >= HOLD_MS {
                Some((hold.from_ms, qualified))
            } else if page.decision.is_none() && hold.to_ms.is_none() {
                Some((hold.from_ms, end))
            } else {
                None
            }
        })
        .collect()
}

fn as_of(page: &DayPage<'_>) -> u64 {
    page.chrome
        .as_of_ms
        .clamp(page.day.open_ms(), page.day.close_ms())
}

fn push_numbers(out: &mut String, page: &DayPage<'_>, live: usize) {
    let until = as_of(page);
    let cast = votes_cast(page.votes, page.day.open_ms(), until);
    let changes: usize = vote_changes(page.votes).values().sum();
    let fetched: usize = fetches(page.evidence).values().sum();
    out.push_str("<section class=\"numbers\" aria-label=\"The day in numbers\">\n");
    push_stat(out, live, "live voters", LIVE_RULE);
    push_stat(out, cast, "votes cast", VOTES_CAST_RULE);
    push_stat(out, changes, "vote changes", VOTE_CHANGES_RULE);
    push_stat(out, fetched, "fetches", FETCHES_RULE);
    push_stat(out, page.arguments.len(), "arguments", ARGUMENTS_RULE);
    out.push_str("</section>\n");
}

fn push_stat(out: &mut String, number: usize, label: &str, rule: &str) {
    out.push_str("<div class=\"card\"><span class=\"n\">");
    out.push_str(&number.to_string());
    out.push_str("</span><span class=\"label\">");
    push_escaped(out, label);
    out.push_str("</span>");
    push_formula(out, rule);
    out.push_str("</div>\n");
}

fn push_formula(out: &mut String, rule: &str) {
    out.push_str("<details class=\"formula\"><summary>formula</summary><p class=\"mono\">");
    out.push_str(rule);
    out.push_str("</p></details>");
}

struct ThreadItem {
    at_ms: u64,
    rank: u8,
    tie: String,
    body: String,
}

fn push_thread(out: &mut String, page: &DayPage<'_>, live: &[String]) {
    let mut items = Vec::new();
    push_argument_items(&mut items, page);
    push_change_items(&mut items, page);
    push_hold_items(&mut items, page, live);
    if let Some(Decision::Decided {
        colour, held_to_ms, ..
    }) = page.decision
    {
        let mut body = thread_start(held_to_ms, Some(colour));
        body.push_str("<p class=\"mark\">");
        push_escaped(&mut body, colour.name());
        body.push_str(" held more than half for a continuous hour. Decided: ");
        push_escaped(&mut body, colour.name());
        body.push_str(".</p></div></li>\n");
        items.push(ThreadItem {
            at_ms: held_to_ms,
            rank: 3,
            tie: colour.name().to_owned(),
            body,
        });
    }
    items.sort_by(|left, right| {
        (left.at_ms, left.rank, left.tie.as_str()).cmp(&(
            right.at_ms,
            right.rank,
            right.tie.as_str(),
        ))
    });
    out.push_str("<section class=\"card\" aria-labelledby=\"thread-h\">\n<h2 id=\"thread-h\">The whole day, oldest first</h2>\n<p class=\"muted\">Shown: arguments, vote changes and the moments the counts crossed half.</p>\n<ol class=\"timeline\">\n");
    for item in items {
        out.push_str(&item.body);
    }
    out.push_str("</ol>\n</section>\n");
}

fn thread_start(at_ms: u64, colour: Option<Colour>) -> String {
    let mut body = String::from("<li><span class=\"t\">");
    body.push_str(&hhmm(at_ms));
    body.push_str(
        "</span><span class=\"rail\" aria-hidden=\"true\"><span class=\"dot\" style=\"background:",
    );
    body.push_str(&colour.map(hex).unwrap_or_else(|| "#E4E3DE".to_owned()));
    body.push_str("\"></span><span class=\"line\"></span></span><div class=\"body\">");
    body
}

fn push_argument_items(items: &mut Vec<ThreadItem>, page: &DayPage<'_>) {
    let mut arguments: Vec<_> = page.arguments.iter().collect();
    arguments.sort_by(|left, right| {
        (left.at_ms, left.author.as_str(), left.id.as_str()).cmp(&(
            right.at_ms,
            right.author.as_str(),
            right.id.as_str(),
        ))
    });
    for argument in arguments {
        let colour = current_votes(page.votes, argument.at_ms)
            .get(&argument.author)
            .copied();
        let mut body = thread_start(argument.at_ms, colour);
        body.push_str("<p>");
        body.push_str(&peer(&page.chrome, &argument.author));
        body.push_str(" argued, voting ");
        if let Some(colour) = colour {
            body.push_str(&chip(colour));
        }
        body.push_str("</p>\n<blockquote>");
        push_escaped(&mut body, &argument.text);
        body.push_str("</blockquote>\n<p><code>");
        push_escaped(&mut body, &argument.id);
        body.push_str("</code></p>\n");
        if argument.cites.is_empty() {
            body.push_str("<p class=\"cite\">cites no evidence</p>\n");
        } else {
            for cite in &argument.cites {
                body.push_str("<p><a class=\"cite\" href=\"#fact-");
                push_escaped(&mut body, &cite.id().0);
                body.push_str("\">");
                push_escaped(&mut body, cite.url().as_str());
                body.push(' ');
                push_escaped(&mut body, cite.excerpt());
                body.push_str("</a></p>\n");
            }
        }
        body.push_str("</div></li>\n");
        items.push(ThreadItem {
            at_ms: argument.at_ms,
            rank: 0,
            tie: format!("{} {}", argument.author, argument.id),
            body,
        });
    }
}

fn push_change_items(items: &mut Vec<ThreadItem>, page: &DayPage<'_>) {
    for (author, trail) in trails(page.votes) {
        let mut previous = None;
        for (at_ms, colour) in trail {
            if let Some(from) = previous.filter(|from| *from != colour) {
                let mut body = thread_start(at_ms, Some(colour));
                body.push_str("<p>");
                body.push_str(&peer(&page.chrome, &author));
                body.push_str(" changed vote ");
                body.push_str(&chip(from));
                body.push_str(" <span aria-hidden=\"true\">→</span> ");
                body.push_str(&chip(colour));
                body.push_str("</p></div></li>\n");
                items.push(ThreadItem {
                    at_ms,
                    rank: 1,
                    tie: author.clone(),
                    body,
                });
            }
            previous = Some(colour);
        }
    }
}

fn push_hold_items(items: &mut Vec<ThreadItem>, page: &DayPage<'_>, live: &[String]) {
    let runs = holds(page.votes, live, page.day.open_ms(), page.day.close_ms());
    for hold in runs {
        let current = current_votes(page.votes, hold.from_ms);
        let holding = live
            .iter()
            .filter(|author| current.get(*author) == Some(&hold.colour))
            .count();
        let mut body = thread_start(hold.from_ms, Some(hold.colour));
        body.push_str("<p class=\"mark\">");
        push_escaped(&mut body, hold.colour.name());
        body.push_str(" holds ");
        body.push_str(&holding.to_string());
        body.push_str(" of ");
        body.push_str(&live.len().to_string());
        body.push_str(", more than half. The hour starts.</p></div></li>\n");
        items.push(ThreadItem {
            at_ms: hold.from_ms,
            rank: 2,
            tie: hold.colour.name().to_owned(),
            body,
        });
    }
}

fn push_facts(out: &mut String, page: &DayPage<'_>) {
    let cited = citation_counts(page.arguments);
    let mut facts: Vec<&Evidence> = page.evidence.iter().collect();
    facts.sort_by(|left, right| {
        (left.fetched_at_ms(), left.id().0.as_str())
            .cmp(&(right.fetched_at_ms(), right.id().0.as_str()))
    });
    out.push_str(
        "<section aria-labelledby=\"facts-h\">\n<h2 id=\"facts-h\">The day\u{2019}s facts</h2>\n",
    );
    out.push_str("<div style=\"display:flex;flex-wrap:wrap;gap:12px\">");
    for fact in facts {
        let id = fact.id().0.as_str();
        out.push_str("<article class=\"card fact\" id=\"fact-");
        push_escaped(out, id);
        out.push_str("\" style=\"flex:1 1 220px\"><div class=\"meta\"><span>");
        let source = match fact.url().host_str() {
            Some(WIKIMEDIA_HOST) => "Wikimedia · On this day",
            Some(SUNRISE_HOST) => "sunrise-sunset.org · London",
            Some(host) => host,
            None => "source",
        };
        push_escaped(out, source);
        out.push_str(" · ev ");
        push_escaped(out, &id.chars().take(8).collect::<String>());
        out.push_str("</span></div><p class=\"excerpt\">");
        push_escaped(out, fact.excerpt());
        out.push_str(
            "</p><p class=\"mono muted\" style=\"font-size:12px;margin:0;overflow-wrap:anywhere\">",
        );
        push_escaped(out, fact.url().as_str());
        out.push_str("</p><div class=\"foot\"><span>fetched by ");
        out.push_str(&peer(&page.chrome, fact.author()));
        out.push_str("</span><span>");
        match cited.get(id).copied().unwrap_or(0) {
            0 => out.push_str("not cited"),
            1 => out.push_str("cited once"),
            count => out.push_str(&format!("cited {count} times")),
        }
        out.push_str("</span></div></article>\n");
    }
    out.push_str("</div></section>\n");
}

fn citation_counts(arguments: &[super::ArgumentView]) -> BTreeMap<String, usize> {
    citations(
        arguments
            .iter()
            .map(|argument| argument.cites.iter().map(Evidence::id)),
    )
}

fn colours_present(points: &[(u64, BTreeMap<Colour, usize>)]) -> Vec<Colour> {
    Colour::ALL
        .into_iter()
        .filter(|colour| {
            points
                .iter()
                .any(|(_, counts)| counts.get(colour).copied().unwrap_or(0) > 0)
        })
        .collect()
}

fn curve_label(page: &DayPage<'_>) -> String {
    let mut label = String::from("Hourly vote counts by colour from 01:00 to 23:00 UTC");
    match page.decision {
        Some(Decision::Decided {
            colour,
            held_from_ms,
            held_to_ms,
        }) => {
            label.push_str("; ");
            label.push_str(colour.name());
            label.push_str(" held from ");
            label.push_str(&hhmm(held_from_ms));
            label.push_str(" to ");
            label.push_str(&hhmm(held_to_ms));
        }
        Some(Decision::Undecided) => {
            label.push_str("; no colour held for a continuous hour");
        }
        None => {
            label.push_str("; the day is open and hours after ");
            label.push_str(&hhmm(as_of(page)));
            label.push_str(" are not yet counted.");
        }
    }
    label
}

fn chart_left(at_ms: u64, columns: usize) -> f64 {
    let seconds = ((at_ms / 1000) % 86_400) as f64;
    let hours = seconds / 3_600.0;
    if columns == 0 {
        0.0
    } else {
        (hours - 0.5) / columns as f64 * 100.0
    }
}

fn hour_label(at_ms: u64) -> String {
    let hour = ((at_ms / 1000) % 86_400) / 3_600;
    if hour == 1 || hour.is_multiple_of(3) {
        format!("{hour:02}")
    } else {
        String::new()
    }
}

fn percent(count: usize, total: usize) -> String {
    if total == 0 {
        return "0.000".to_owned();
    }
    let millis = count.saturating_mul(100_000) / total;
    format!("{}.{:03}", millis / 1_000, millis % 1_000)
}

fn capitalise(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn long_date(day: Day) -> String {
    let (_, number) = day.month_day();
    format!(
        "{} {number} {} {}",
        day.weekday_name(),
        day.month_name(),
        year_of(day)
    )
}

fn prose_date(day: Day) -> String {
    let (_, number) = day.month_day();
    format!("{} {number} {}", day.weekday_name(), day.month_name())
}

fn status_line(day: Day, open: bool) -> String {
    let (_, number) = day.month_day();
    let status = if open {
        "OPEN UNTIL 23:00 UTC"
    } else {
        "CLOSED 23:00 UTC"
    };
    format!(
        "{} {number} {} {} · {status}",
        day.weekday_name().to_ascii_uppercase(),
        day.month_name().to_ascii_uppercase(),
        year_of(day)
    )
}

fn year_of(day: Day) -> String {
    day.to_string().split('-').next().unwrap_or("").to_owned()
}

fn hex(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!("#{red:02X}{green:02X}{blue:02X}")
}

fn hhmm(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) % 86_400;
    let hour = secs / 3_600;
    let minute = (secs % 3_600) / 60;
    format!("{hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use url::Url;

    use super::render_day;
    use crate::closing::{Decision, HOLD_MS, LIVE_RULE, RULE};
    use crate::colour::Colour::{self, Blue, Red};
    use crate::day::Day;
    use crate::evidence::Evidence;
    use crate::site::support::{ESCAPED, RAW, assert_clean, body_open, page, vote};
    use crate::site::{ArgumentView, Chrome, DayPage, Paint};
    use crate::tally::{
        ARGUMENTS_RULE, CURVE_RULE, FETCHES_RULE, VOTE_CHANGES_RULE, VOTES_CAST_RULE, curve,
    };

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
        assert!(html.contains("Blue."));
        assert!(html.contains("<h1 class=\"verdict\">Blue.</h1>"));
        assert!(!html.contains("font-size:132px"));
        assert!(html.contains("from 00:00 to 01:00 UTC"));
        assert!(html.contains("It painted Friday 2 January."));
        assert!(html.contains(RULE));
        assert!(html.contains(LIVE_RULE));
        assert!(html.contains(CURVE_RULE));
        assert!(html.contains(VOTES_CAST_RULE));
        assert!(html.contains(VOTE_CHANGES_RULE));
        assert!(html.contains(FETCHES_RULE));
        assert!(html.contains(ARGUMENTS_RULE));
        assert!(html.contains("held 00:00\u{2013}01:00"));
        assert!(html.contains("blue holds 2 of 2, more than half. The hour starts."));
        assert!(!html.contains("stroke="));
        assert!(!html.contains("<svg"));
        assert!(!html.contains("<polyline"));
        assert_eq!(html.matches("data-hour=\"").count(), 23);
        assert_eq!(html.matches("<tr><td>").count(), 23);
        assert_clean(&html);
    }

    #[test]
    fn render_day_undecided_sets_no_background() {
        let shown = page(&[], &[], &[], Some(Decision::Undecided), Paint::Undecided);
        let html = render_day(&shown);
        assert!(html.contains("Undecided."));
        assert!(html.contains("<h1 class=\"verdict\">Undecided.</h1>"));
        assert!(!html.contains("font-size:132px"));
        assert!(html.contains("Friday 2 January was left unpainted."));
        assert!(html.contains("The rule that decided it"));
        assert_eq!(body_open(&html), "<body>");
        assert_clean(&html);
    }

    #[test]
    fn a_live_day_says_open_and_has_no_next_link() {
        let shown = page(&[], &[], &[], None, Paint::NoRecord);
        let html = render_day(&shown);
        assert!(html.contains("Open."));
        assert!(html.contains("<h1 class=\"verdict\">Open.</h1>"));
        assert!(!html.contains("font-size:132px"));
        assert!(html.contains("The question is open until 23:00 UTC."));
        assert!(html.contains("The rule that will decide it"));
        assert!(html.contains("OPEN UNTIL 23:00 UTC"));
        assert!(!html.contains("Today →"));
        assert!(!html.contains("1970-01-02.html"));
        assert!(!html.contains("<nav aria-label=\"Days\">"));
        assert_clean(&html);
    }

    #[test]
    fn navigation_uses_only_supplied_neighbours_and_their_outcomes() {
        let day: Day = "2026-10-25".parse().unwrap();
        let previous: Day = "2026-10-24".parse().unwrap();
        let next: Day = "2026-10-26".parse().unwrap();
        let mut shown = page(&[], &[], &[], Some(Decision::Undecided), Paint::Undecided);
        shown.day = day;
        shown.chrome.as_of_ms = next.open_ms();
        shown.previous = Some((previous, Paint::Undecided));
        shown.next = Some((next, Paint::NoRecord));
        let html = render_day(&shown);
        assert!(html.contains("<a href=\"2026-10-24.html\">← Saturday 24 October · undecided</a>"));
        assert!(html.contains("<a href=\"../index.html\">Monday 26 October · open now →</a>"));

        shown.previous = None;
        let html = render_day(&shown);
        assert!(!html.contains('←'));
        assert!(html.contains("Monday 26 October · open now →"));
        shown.next = None;
        assert!(!render_day(&shown).contains("<nav aria-label=\"Days\">"));

        // Recorded neighbours need not be consecutive calendar days.
        shown.chrome.as_of_ms = next.next().open_ms();
        for (paint, label) in [
            (Paint::Decided(Blue), "blue"),
            (Paint::Undecided, "undecided"),
            (Paint::NoRecord, "no record"),
        ] {
            shown.previous = Some((previous.previous(), paint));
            shown.next = Some((next, paint));
            let html = render_day(&shown);
            assert!(html.contains(&format!(
                "<a href=\"2026-10-23.html\">← Friday 23 October · {label}</a>"
            )));
            assert!(html.contains(&format!(
                "<a href=\"2026-10-26.html\">Monday 26 October · {label} →</a>"
            )));
            assert_markup_clean(&html);
        }

        // Even a supplied next neighbour cannot put a next link on today.
        shown.chrome.as_of_ms = day.open_ms();
        let html = render_day(&shown);
        assert!(html.contains('←'));
        assert!(!html.contains(" →</a>"));
    }

    #[test]
    fn a_blue_afternoon_stacks_the_curve_and_one_hold() {
        let day: Day = "2026-10-25".parse().unwrap();
        let today: Day = "2026-10-26".parse().unwrap();
        let from = day.open_ms() + 17 * 3_600_000 + 10 * 60 * 1000;
        let mut votes = Vec::new();
        for index in 0..5 {
            votes.push(vote(
                &format!("p{index:02}"),
                Red,
                day.open_ms() + 3_600_000,
            ));
        }
        for index in 5..12 {
            votes.push(vote(&format!("p{index:02}"), Blue, from));
        }
        let names = BTreeMap::new();
        let shown = DayPage {
            day,
            previous: Some((day.previous(), Paint::Undecided)),
            next: Some((today, Paint::NoRecord)),
            votes: &votes,
            arguments: &[],
            evidence: &[],
            decision: Some(Decision::Decided {
                colour: Blue,
                held_from_ms: from,
                held_to_ms: from + HOLD_MS,
            }),
            chrome: Chrome {
                painted_by: Paint::Decided(Blue),
                as_of_ms: today.open_ms(),
                rendered_at_ms: today.open_ms(),
                server: "peer-a",
                names: &names,
            },
        };
        let html = render_day(&shown);
        assert!(html.contains("Blue."));
        assert!(html.contains("from 17:10 to 18:10 UTC"));
        assert!(html.contains("It painted Monday 26 October."));
        assert!(html.contains("SUNDAY 25 OCTOBER 2026 · CLOSED 23:00 UTC"));
        assert!(html.contains("Monday 26 October · open now →"));
        assert!(html.contains("href=\"2026-10-24.html\""));
        assert!(!html.contains("href=\"2026-10-26.html\""));
        assert_eq!(html.matches("held 17:10\u{2013}18:10").count(), 1);
        assert_eq!(html.matches("data-hour=\"").count(), 23);
        assert_eq!(html.matches("<tr><td>").count(), 23);
        assert!(html.contains("aria-label=\"Hourly vote counts by colour from 01:00 to 23:00 UTC; blue held from 17:10 to 18:10\""));

        assert!(!html.contains("data-colour=\"not-yet\""));
        assert!(!html.contains("</span>not yet</span>"));
        assert!(!html.contains("outline:2.5px dashed"));
        assert!(html.contains("width:4.35%;box-shadow:0 0 0 2.5px #141414"));

        let points = curve(&votes, day.open_ms(), day.close_ms());
        assert_eq!(points.len(), 23);
        let at = day.open_ms() + 18 * 3_600_000;
        let counts = &points
            .iter()
            .find(|(stamp, _)| *stamp == at)
            .expect("18:00")
            .1;
        let column = column_at(&html, "18:00");
        let mut seen = 0;
        for colour in Colour::ALL {
            let count = counts.get(&colour).copied().unwrap_or(0);
            if count == 0 {
                continue;
            }
            seen += 1;
            let millis = count * 100_000 / 12;
            let height = format!("{}.{:03}", millis / 1_000, millis % 1_000);
            let needle = format!(
                "data-colour=\"{}\" style=\"height:{height}%;",
                colour.name()
            );
            assert!(column.contains(&needle), "missing {needle} in {column}");
        }
        assert_eq!(seen, 2);
        assert!(!column.contains("not-voted-yet"));
        assert_markup_clean(&html);
    }

    #[test]
    fn an_open_day_draws_only_observed_hours_and_bounds_the_hold_by_now() {
        let from = 13 * HOLD_MS + 37 * 60_000;
        let votes = [vote("ada", Blue, from)];
        let mut shown = page(&votes, &[], &[], None, Paint::NoRecord);
        for (now, first_future, complete) in [
            (from + 60_000, 14, false),
            (14 * HOLD_MS, 15, false),
            (14 * HOLD_MS + 37 * 60_000, 15, true),
            (14 * HOLD_MS + 40 * 60_000, 15, true),
        ] {
            shown.chrome.as_of_ms = now;
            let html = render_day(&shown);
            assert_eq!(
                html.matches("data-colour=\"not-yet\"").count(),
                24 - first_future
            );
            for hour in 1..=23 {
                let column = column_at(&html, &format!("{hour:02}:00"));
                if hour >= first_future {
                    assert_eq!(column.matches("data-colour=").count(), 1);
                    assert!(column.contains(&format!(
                        "data-colour=\"not-yet\" style=\"height:100%;background:{}\"",
                        super::NOT_YET
                    )));
                    assert!(html.contains(&format!("<tr><td>{hour:02}:00</td><td>–</td></tr>")));
                } else {
                    assert!(!column.contains("data-colour=\"not-yet\""));
                    let count = usize::from(hour * HOLD_MS as usize >= from as usize);
                    assert!(
                        html.contains(&format!("<tr><td>{hour:02}:00</td><td>{count}</td></tr>"))
                    );
                }
            }
            assert!(html.contains("</span>not yet</span>"));
            assert!(html.contains(&format!(
                "; the day is open and hours after {} are not yet counted.\"",
                super::hhmm(now)
            )));
            if complete {
                assert!(html.contains("held 13:37–14:37"));
                assert!(html.contains("width:4.35%;box-shadow:0 0 0 2.5px #141414"));
                assert!(!html.contains("holding since"));
                assert!(!html.contains("outline:2.5px dashed"));
            } else {
                assert!(html.contains("holding since 13:37"));
                assert!(!html.contains("held 13:37–14:37"));
                let width = (now - from) as f64 / HOLD_MS as f64 * 100.0 / 23.0;
                assert!(html.contains(&format!("width:{width:.2}%;outline:2.5px dashed #141414")));
            }
            assert_markup_clean(&html);
        }
    }

    #[test]
    fn open_day_cutoff_clamps_to_both_day_edges() {
        let votes = [vote("ada", Blue, 0)];
        let mut shown = page(&votes, &[], &[], None, Paint::NoRecord);
        shown.day = "1970-01-02".parse().unwrap();
        let html = render_day(&shown);
        assert_eq!(html.matches("data-colour=\"not-yet\"").count(), 23);
        assert!(html.contains("hours after 00:00 are not yet counted."));
        assert!(!html.contains("holding since"));
        shown.chrome.as_of_ms = shown.day.close_ms() + HOLD_MS;
        let html = render_day(&shown);
        assert!(!html.contains("data-colour=\"not-yet\""));
        assert!(!html.contains("</span>not yet</span>"));
        assert!(html.contains("hours after 23:00 are not yet counted."));
    }

    #[test]
    fn a_short_interrupted_hold_does_not_get_an_outline_or_label() {
        let votes = [
            vote("ada", Blue, 13 * HOLD_MS),
            vote("ada", Red, 13 * HOLD_MS + 20 * 60_000),
        ];
        let mut shown = page(&votes, &[], &[], None, Paint::NoRecord);
        shown.chrome.as_of_ms = 13 * HOLD_MS + 30 * 60_000;
        let html = render_day(&shown);
        assert!(html.contains("holding since 13:20"));
        assert!(!html.contains("holding since 13:00"));
        assert!(!html.contains("<p>held "));
        assert_eq!(html.matches("outline:2.5px dashed #141414").count(), 1);
    }

    #[test]
    fn facts_use_source_labels_and_zero_one_many_citation_phrases() {
        let records = [
            Evidence::new(
                "ada".to_owned(),
                "run".to_owned(),
                Url::parse("https://api.wikimedia.org/fact").unwrap(),
                0,
                "wiki",
            ),
            Evidence::new(
                "bea".to_owned(),
                "run".to_owned(),
                Url::parse("https://api.sunrise-sunset.org/fact").unwrap(),
                0,
                "sunset",
            ),
            Evidence::new(
                "cy".to_owned(),
                "run".to_owned(),
                Url::parse("https://example.com/fact").unwrap(),
                0,
                "other",
            ),
        ];
        let arguments = [
            ArgumentView {
                id: "a".to_owned(),
                author: "ada".to_owned(),
                at_ms: 0,
                text: "first".to_owned(),
                cites: vec![records[0].clone(), records[1].clone()],
            },
            ArgumentView {
                id: "b".to_owned(),
                author: "bea".to_owned(),
                at_ms: 0,
                text: "second".to_owned(),
                cites: vec![records[1].clone()],
            },
        ];
        let html = render_day(&page(&[], &arguments, &records, None, Paint::NoRecord));
        for (fact, label, phrase) in [
            (&records[0], "Wikimedia · On this day", "cited once"),
            (&records[1], "sunrise-sunset.org · London", "cited 2 times"),
            (&records[2], "example.com", "not cited"),
        ] {
            let marker = format!("<article class=\"card fact\" id=\"fact-{}\"", fact.id().0);
            let article = html
                .split(&marker)
                .nth(1)
                .unwrap()
                .split("</article>")
                .next()
                .unwrap();
            assert!(article.contains(label));
            assert!(article.contains(phrase));
        }
        assert_clean(&html);
    }

    #[test]
    fn the_thread_lists_arguments_changes_and_markers_oldest_first() {
        let day = Day::from_ms(0);
        let votes = [
            vote("ada", Red, 3_600_000),
            vote("ada", Blue, 3 * 3_600_000),
        ];
        let arguments = [ArgumentView {
            id: "arg-9".to_owned(),
            author: "ada".to_owned(),
            at_ms: 2 * 3_600_000,
            text: "hello".to_owned(),
            cites: Vec::new(),
        }];
        let names = BTreeMap::new();
        let shown = DayPage {
            day,
            previous: None,
            next: None,
            votes: &votes,
            arguments: &arguments,
            evidence: &[],
            decision: Some(Decision::Decided {
                colour: Red,
                held_from_ms: 3_600_000,
                held_to_ms: 2 * 3_600_000,
            }),
            chrome: Chrome {
                painted_by: Paint::Decided(Red),
                as_of_ms: 0,
                rendered_at_ms: 3_723_000,
                server: "peer-a",
                names: &names,
            },
        };
        let html = render_day(&shown);
        let first_hold = html.find("The hour starts").expect("first hold");
        let hello = html.find("hello").expect("argument");
        let decided = html
            .find("continuous hour. Decided:")
            .expect("decided marker");
        let change = html.find("changed vote").expect("vote change");
        let second_hold = html[first_hold + 1..]
            .find("The hour starts")
            .expect("second hold")
            + first_hold
            + 1;
        assert!(first_hold < hello && hello < decided && decided < change && change < second_hold);
        assert_eq!(html.matches("changed vote").count(), 1);
        assert!(html.contains("cites no evidence"));
        assert!(html.contains("red holds 1 of 1, more than half. The hour starts."));
        assert!(html.contains("blue holds 1 of 1, more than half. The hour starts."));
        let thread = html
            .split("<ol class=\"timeline\">")
            .nth(1)
            .unwrap()
            .split("</ol>")
            .next()
            .unwrap();
        assert_eq!(thread.matches("<li>").count(), 5);
        for class in ["t", "rail", "dot", "line", "body"] {
            assert_eq!(thread.matches(&format!("class=\"{class}\"")).count(), 5);
        }
        assert_eq!(thread.matches("class=\"mark\"").count(), 3);
        assert!(thread.contains("<blockquote>hello</blockquote>"));
        assert!(thread.contains("class=\"cite\""));
        assert_clean(&html);
    }

    #[test]
    fn facts_anchor_every_record_and_count_citations() {
        let evidence = Evidence::new(
            "ada".to_owned(),
            "run".to_owned(),
            Url::parse("https://example.com/fact").unwrap(),
            0,
            &format!("a fact\n{RAW}"),
        );
        let arguments = [ArgumentView {
            id: "arg-1".to_owned(),
            author: "ada".to_owned(),
            at_ms: 0,
            text: "because".to_owned(),
            cites: vec![evidence.clone()],
        }];
        let records = [evidence.clone()];
        let names = BTreeMap::new();
        let shown = DayPage {
            day: Day::from_ms(0),
            previous: None,
            next: None,
            votes: &[],
            arguments: &arguments,
            evidence: &records,
            decision: None,
            chrome: Chrome {
                painted_by: Paint::NoRecord,
                as_of_ms: 0,
                rendered_at_ms: 3_723_000,
                server: "peer-a",
                names: &names,
            },
        };
        let html = render_day(&shown);
        let id = evidence.id().0.as_str();
        assert!(html.contains(&format!("id=\"fact-{id}\"")));
        assert!(html.contains(&format!("href=\"#fact-{id}\"")));
        assert!(html.contains("a fact"));
        assert!(html.contains("cited once"));
        assert!(html.contains("https://example.com/fact"));
        assert!(html.contains(&format!(
            "<article class=\"card fact\" id=\"fact-{id}\" style=\"flex:1 1 220px\">"
        )));
        assert!(html.contains(&format!(
            "<div class=\"meta\"><span>example.com · ev {}",
            &id[..8]
        )));
        assert!(html.contains(&format!("<p class=\"excerpt\">a fact\n{ESCAPED}</p>")));
        assert!(html.contains("overflow-wrap:anywhere\">https://example.com/fact</p>"));
        assert!(html.contains("<div class=\"foot\"><span>fetched by "));
        assert!(html.contains("<section class=\"hero\"><div class=\"hero-text\">"));
        assert!(html.contains("class=\"mono eyebrow\""));
        assert!(html.contains("<p class=\"lede\">"));
        assert!(html.contains("<strong>The rule that will decide it</strong>"));
        let numbers = html
            .split("<section class=\"numbers\" aria-label=\"The day in numbers\">")
            .nth(1)
            .unwrap()
            .split("</section>")
            .next()
            .unwrap();
        assert_eq!(
            numbers
                .matches("<div class=\"card\"><span class=\"n\">")
                .count(),
            5
        );
        assert_eq!(numbers.matches("</span><span class=\"label\">").count(), 5);
        assert_eq!(numbers.matches("class=\"formula\"").count(), 5);
        let legend = html
            .split("<div class=\"legend\">")
            .nth(1)
            .unwrap()
            .split("</div>")
            .next()
            .unwrap();
        assert_eq!(legend.matches("<span><span class=\"swatch-xs\"").count(), 2);
        assert_clean(&html);
    }

    fn column_at<'a>(html: &'a str, hour: &str) -> &'a str {
        let marker = format!("data-hour=\"{hour}\"");
        let start = html
            .find(&marker)
            .unwrap_or_else(|| panic!("missing {hour}"));
        let rest = &html[start..];
        let end = rest[marker.len()..]
            .find("data-hour=")
            .map(|at| at + marker.len())
            .unwrap_or(rest.len());
        &rest[..end]
    }

    fn assert_markup_clean(html: &str) {
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"));
        assert!(!lower.contains("<link"));
        assert!(!lower.contains("src="));
        for word in crate::site::support::FORBIDDEN {
            assert!(!lower.contains(word), "{word}");
        }
    }
}
