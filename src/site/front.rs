//! Today's page: what the swarm is choosing right now.

use std::collections::BTreeMap;

use crate::closing::{
    Decision, HOLD_MS, HOLDING_RULE, LIVE_RULE, RULE, decide, holding, live_voters,
};
use crate::colour::Colour;
use crate::day::{Day, utc_time};
use crate::evidence::{Evidence, SUNRISE_HOST, WIKIMEDIA_HOST};
use crate::tally::{
    ARGUMENTS_RULE, CITATIONS_RULE, COUNTS_RULE, CURRENT_VOTES_RULE, FETCHES_RULE,
    VOTE_CHANGES_RULE, VOTES_CAST_RULE, citations, counts, current_votes, trails, vote_changes,
    votes_cast,
};

use super::layout::{chip, document, formula, peer, push_escaped};
use super::{DayPage, Paint, Section};

const HOUR_MINUTES: u64 = HOLD_MS / 60_000;
const FACTS_SHOWN: usize = 12;

/// Today's page at `chrome.as_of_ms`, on the shared shell.
///
/// `decide`'s window ends at that instant, so a hold still short of an hour
/// stays on the bar. Passing the day's close would treat the hour as already
/// finished.
pub fn render_front(page: &DayPage<'_>) -> String {
    let now = page.chrome.as_of_ms;
    let open = page.day.open_ms();
    let live = live_voters(page.votes, open, page.day.close_ms());
    let mut body = String::new();
    push_hero(&mut body, page, live.len());
    push_right_now(&mut body, page, &live, now, open);
    body.push_str("</section>\n");
    push_current(&mut body, page, now);
    push_numbers(&mut body, page, live.len(), now, open);
    push_activity(&mut body, page, &live, now, open);
    let title = format!("Today {}", page.day);
    document(&title, Some(Section::Today), 0, &page.chrome, &body)
}

fn push_hero(out: &mut String, page: &DayPage<'_>, live: usize) {
    out.push_str("<section class=\"hero\"><div class=\"hero-text\"><p class=\"mono eyebrow\" style=\"margin:0\">");
    push_escaped(out, &date_line(&page.day));
    out.push_str("</p>\n<h1 style=\"margin:0\">");
    push_escaped(out, &heading(live));
    out.push_str("</h1>\n<p class=\"lede\" style=\"margin:0\">");
    push_escaped(out, &paint_sentence(page.chrome.painted_by));
    out.push(' ');
    out.push_str(
        "Each peer reads a few facts about the date, argues in short messages and votes. \
         The first colour held by more than half of them for an hour paints this page tomorrow.",
    );
    out.push_str("</p>\n</div>\n");
}

fn date_line(day: &Day) -> String {
    let rendered = day.to_string();
    let year = rendered.split('-').next().unwrap_or(&rendered);
    let (_, day_of_month) = day.month_day();
    format!(
        "{} {} {} {} · OPEN {}–{} UTC",
        day.weekday_name(),
        day_of_month,
        day.month_name(),
        year,
        hhmm(day.open_ms()),
        hhmm(day.close_ms()),
    )
    .to_uppercase()
}

fn heading(live: usize) -> String {
    if live >= 2 {
        format!("{live} small models are choosing tomorrow\u{2019}s colour.")
    } else {
        "The swarm is choosing tomorrow\u{2019}s colour.".to_owned()
    }
}

fn paint_sentence(paint: Paint) -> String {
    match paint {
        Paint::Decided(colour) => format!(
            "This page is {} because yesterday the swarm decided {}.",
            colour.name(),
            colour.name()
        ),
        Paint::Undecided => {
            "Yesterday was undecided under the rule, so today this page is unpainted.".to_owned()
        }
        Paint::NoRecord => {
            "No result is recorded for yesterday, so today this page is unpainted.".to_owned()
        }
    }
}

fn push_right_now(out: &mut String, page: &DayPage<'_>, live: &[String], now: u64, open: u64) {
    out.push_str("<section class=\"card hero-card\"><p class=\"mono muted\" style=\"margin:0;font-size:13px\">RIGHT NOW · ");
    out.push_str(&hhmm(now));
    out.push_str(" UTC</p>\n");
    match decide(page.votes, live, open, now) {
        Decision::Decided {
            colour,
            held_from_ms,
            held_to_ms,
        } => push_decided(out, colour, held_from_ms, held_to_ms),
        Decision::Undecided => match holding(page.votes, live, open, now) {
            Some(hold) => push_holding(out, page, live, now, &hold),
            None => push_no_hold(out, page, live.len(), now),
        },
    }
    out.push_str("<details style=\"border-top:1px solid var(--hairline);padding-top:14px\"><summary>The closing rule</summary><p class=\"mono\">");
    push_escaped(out, RULE);
    out.push_str("</p>");
    out.push_str(&formula(HOLDING_RULE));
    out.push_str("</details>\n</section>\n");
}

fn push_swatch(out: &mut String, colour: Colour) {
    out.push_str("<span class=\"swatch\" aria-hidden=\"true\" style=\"background:");
    out.push_str(&hex(colour));
    out.push_str("\"></span>");
}

fn push_colour_name(out: &mut String, colour: Colour) {
    out.push_str("<span class=\"big\">");
    let name = colour.name();
    out.push_str(&name[..1].to_ascii_uppercase());
    out.push_str(&name[1..]);
    out.push_str("</span>");
}

fn push_decided(out: &mut String, colour: Colour, from_ms: u64, to_ms: u64) {
    out.push_str("<div class=\"chips\">Decided: ");
    push_swatch(out, colour);
    push_colour_name(out, colour);
    out.push_str(", held ");
    out.push_str(&hhmm(from_ms));
    out.push('\u{2013}');
    out.push_str(&hhmm(to_ms));
    out.push_str(" UTC. It paints this page tomorrow.</div>\n");
}

fn push_holding(
    out: &mut String,
    page: &DayPage<'_>,
    live: &[String],
    now: u64,
    hold: &crate::closing::Hold,
) {
    let current = current_votes(page.votes, now);
    let count = live
        .iter()
        .filter(|author| current.get(author.as_str()) == Some(&hold.colour))
        .count();
    let capped = now.saturating_sub(hold.from_ms).min(HOLD_MS);
    let minutes = capped / 60_000;
    let deadline = hhmm(hold.from_ms.saturating_add(HOLD_MS));
    out.push_str("<div style=\"display:flex;align-items:center;gap:16px\">");
    push_swatch(out, hold.colour);
    out.push_str("<div style=\"display:flex;flex-direction:column\">");
    push_colour_name(out, hold.colour);
    out.push_str("<span>");
    out.push_str(&count.to_string());
    out.push_str(" of ");
    out.push_str(&live.len().to_string());
    out.push_str(" live voters · more than half needs ");
    out.push_str(&(live.len() / 2 + 1).to_string());
    out.push_str("</span></div></div>\n");
    let label = format!(
        "{} has held more than half for {minutes} of {HOUR_MINUTES} minutes",
        hold.colour.name()
    );
    out.push_str("<div style=\"display:flex;flex-direction:column;gap:8px\"><div class=\"bar\" role=\"img\" aria-label=\"");
    push_escaped(out, &label);
    out.push_str("\"><div class=\"fill\" style=\"width:");
    out.push_str(&width_percent(capped));
    out.push_str(";background:");
    out.push_str(&hex(hold.colour));
    out.push_str("\"></div>");
    for tick in [25, 50, 75] {
        out.push_str(&format!(
            "<div class=\"tick\" style=\"left:{tick}%\"></div>"
        ));
    }
    out.push_str("</div>\n<div class=\"bar-labels mono\"><span>held since ");
    out.push_str(&hhmm(hold.from_ms));
    out.push_str("</span><span>");
    out.push_str(&minutes.to_string());
    out.push_str(" of ");
    out.push_str(&HOUR_MINUTES.to_string());
    out.push_str(" min</span><span>");
    out.push_str(&deadline);
    out.push_str("</span></div></div>\n<p style=\"margin:0;font-size:18px;line-height:1.4\">If no vote moves before <strong>");
    out.push_str(&deadline);
    out.push_str(" UTC</strong>, ");
    out.push_str(hold.colour.name());
    out.push_str(" is the day\u{2019}s colour and paints this page tomorrow.</p>\n");
}

fn push_no_hold(out: &mut String, page: &DayPage<'_>, live: usize, now: u64) {
    let totals = counts(page.votes, now);
    if totals.is_empty() {
        out.push_str("<p>No votes yet today.</p>\n");
        return;
    }
    let most = totals.values().copied().max().unwrap_or(0);
    let mut colours: Vec<_> = totals
        .into_iter()
        .filter(|(_, count)| *count == most)
        .collect();
    colours.sort_by_key(|(colour, _)| *colour);
    out.push_str("<div class=\"chips\">");
    for (index, (colour, count)) in colours.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        push_swatch(out, *colour);
        push_colour_name(out, *colour);
        out.push(' ');
        out.push_str(&count.to_string());
    }
    out.push_str("</div>\n<p>No colour has more than half of the ");
    out.push_str(&live.to_string());
    out.push_str(" live voters.</p>\n");
}

fn push_current(out: &mut String, page: &DayPage<'_>, now: u64) {
    let current = current_votes(page.votes, now);
    let totals = counts(page.votes, now);
    let mut authors_of: BTreeMap<Colour, Vec<&str>> = BTreeMap::new();
    for (author, colour) in &current {
        authors_of.entry(*colour).or_default().push(author);
    }
    for authors in authors_of.values_mut() {
        authors.sort_by(|left, right| {
            display_name(&page.chrome, left)
                .cmp(&display_name(&page.chrome, right))
                .then(left.cmp(right))
        });
    }
    let mut order: Vec<Colour> = totals.keys().copied().collect();
    order.sort_by(|left, right| totals[right].cmp(&totals[left]).then(left.cmp(right)));

    out.push_str("<section class=\"card\" aria-labelledby=\"votes-h\" style=\"display:flex;flex-direction:column;gap:20px\"><div style=\"display:flex;flex-wrap:wrap;justify-content:space-between;align-items:baseline;gap:12px\"><h2 id=\"votes-h\" style=\"margin:0\">Current votes</h2>\n<div class=\"chips\">");
    for colour in &order {
        out.push_str("<span class=\"chip-lg\"><span class=\"swatch-sm\" aria-hidden=\"true\" style=\"background:");
        out.push_str(&hex(*colour));
        out.push_str("\"></span>");
        out.push_str(colour.name());
        out.push(' ');
        out.push_str(&totals[colour].to_string());
        out.push_str("</span>");
    }
    out.push_str("</div></div>\n<ol class=\"tiles\">\n");
    for colour in &order {
        for author in authors_of.get(colour).into_iter().flatten() {
            out.push_str("<li class=\"tile\"><span class=\"paint mono\" style=\"background:");
            out.push_str(&hex(*colour));
            out.push_str(";color:");
            out.push_str(contrasting_text(colour.srgb()));
            out.push_str("\">");
            out.push_str(colour.name());
            out.push_str("</span><a href=\"flock.html\"><span class=\"name\">");
            if let Some(name) = page.chrome.names.get(*author) {
                push_escaped(out, name);
            }
            out.push_str("</span><span class=\"id mono\">");
            push_escaped(out, author);
            out.push_str("</span></a></li>\n");
        }
    }
    out.push_str("</ol>\n<details><summary>How these are counted</summary><p class=\"mono\">");
    push_escaped(out, CURRENT_VOTES_RULE);
    out.push_str("</p><p class=\"mono\">");
    push_escaped(out, COUNTS_RULE);
    out.push_str("</p></details></section>\n");
}

fn push_numbers(out: &mut String, page: &DayPage<'_>, live: usize, now: u64, open: u64) {
    let changes: usize = vote_changes(page.votes).values().sum();
    out.push_str("<section class=\"numbers\" aria-label=\"Today in numbers\">\n");
    push_stat(out, live, "live voters", LIVE_RULE);
    push_stat(
        out,
        votes_cast(page.votes, open, now),
        "votes cast",
        VOTES_CAST_RULE,
    );
    push_stat(out, changes, "vote changes", VOTE_CHANGES_RULE);
    push_stat(out, page.evidence.len(), "fetches", FETCHES_RULE);
    push_stat(out, page.arguments.len(), "arguments", ARGUMENTS_RULE);
    out.push_str("</section>\n");
}

fn push_stat(out: &mut String, number: usize, label: &str, rule: &str) {
    out.push_str("<div class=\"card\"><span class=\"n\">");
    out.push_str(&number.to_string());
    out.push_str("</span><span class=\"label\">");
    push_escaped(out, label);
    out.push_str("</span>");
    out.push_str(&formula(rule));
    out.push_str("</div>\n");
}

fn push_activity(out: &mut String, page: &DayPage<'_>, live: &[String], now: u64, open: u64) {
    out.push_str("<div class=\"split\">\n<section class=\"card main\" aria-labelledby=\"day-h\"><h2 id=\"day-h\">The day so far</h2>\n<p class=\"muted\" style=\"font-size:15px\">Arguments and vote changes, newest first. Every message is the peer’s own words, quoted as data.</p><ol class=\"timeline\">\n");
    for event in timeline(page, live, now, open) {
        match event {
            Event::Argument {
                at_ms,
                author,
                text,
                cites,
                ..
            } => push_argument(out, page, at_ms, author, text, cites),
            Event::Change {
                at_ms,
                author,
                from,
                to,
            } => push_change(out, page, at_ms, &author, from, to),
            Event::Marker {
                at_ms,
                colour,
                count,
            } => push_marker(out, at_ms, colour, count, live.len()),
        }
    }
    out.push_str("</ol>\n<a href=\"days/");
    push_escaped(out, &page.day.to_string());
    out.push_str(".html\">Every record today, oldest first →</a>\n</section>\n");
    push_facts(out, page);
    out.push_str("</div>\n");
}

fn push_argument(
    out: &mut String,
    page: &DayPage<'_>,
    at_ms: u64,
    author: &str,
    text: &str,
    cites: &[Evidence],
) {
    let current = current_votes(page.votes, at_ms);
    push_event_start(out, at_ms, current.get(author).copied());
    out.push_str("<div><strong>");
    out.push_str(&peer(&page.chrome, author));
    out.push_str("</strong> <span class=\"muted\">argued, voting</span> ");
    match current.get(author) {
        Some(colour) => out.push_str(&chip(*colour)),
        None => out.push_str("no vote yet"),
    }
    out.push_str("</div><blockquote>\u{201C}");
    push_escaped(out, text);
    out.push_str("\u{201D}</blockquote><div class=\"chips\">");
    for cite in cites {
        let id = &cite.id().0;
        out.push_str("<a class=\"cite\" href=\"#fact-");
        push_escaped(out, id);
        out.push_str("\">cites ");
        push_escaped(out, &prefix(id));
        out.push_str("</a>");
    }
    out.push_str("</div></div></li>\n");
}

fn push_change(
    out: &mut String,
    page: &DayPage<'_>,
    at_ms: u64,
    author: &str,
    from: Colour,
    to: Colour,
) {
    push_event_start(out, at_ms, Some(to));
    out.push_str("<div><strong>");
    out.push_str(&peer(&page.chrome, author));
    out.push_str("</strong> changed vote ");
    out.push_str(&chip(from));
    out.push_str(" <span aria-hidden=\"true\">→</span> ");
    out.push_str(&chip(to));
    out.push_str("</div></div></li>\n");
}

fn push_marker(out: &mut String, at_ms: u64, colour: Colour, count: usize, live: usize) {
    push_event_start(out, at_ms, Some(colour));
    out.push_str("<div class=\"mark\">");
    out.push_str(colour.name());
    out.push_str(" now holds ");
    out.push_str(&count.to_string());
    out.push_str(" of ");
    out.push_str(&live.to_string());
    out.push_str(", more than half. The hour starts.</div></div></li>\n");
}

fn push_event_start(out: &mut String, at_ms: u64, colour: Option<Colour>) {
    out.push_str("<li><span class=\"t mono\">");
    out.push_str(&hhmm(at_ms));
    out.push_str(
        "</span><span class=\"rail\" aria-hidden=\"true\"><span class=\"dot\" style=\"background:",
    );
    match colour {
        Some(colour) => out.push_str(&hex(colour)),
        None => out.push_str("var(--chip)"),
    }
    out.push_str("\"></span><span class=\"line\"></span></span><div class=\"body\">");
}

fn push_facts(out: &mut String, page: &DayPage<'_>) {
    let groups = fact_groups(page);
    let total = groups.len();
    out.push_str(
        "<section class=\"side\" aria-labelledby=\"facts-h\"><h2 id=\"facts-h\">Today\u{2019}s facts</h2>\n",
    );
    out.push_str("<p style=\"font-size:15px\">What the peers fetched. Each can afford three fetches per activation, from two sources.</p>");
    out.push_str(&formula(CITATIONS_RULE));
    for group in groups.iter().take(FACTS_SHOWN) {
        let first = group.records[0];
        out.push_str("<article class=\"card fact\" id=\"fact-");
        push_escaped(out, &first.id().0);
        out.push_str("\">");
        for record in group.records.iter().skip(1) {
            out.push_str("<a id=\"fact-");
            push_escaped(out, &record.id().0);
            out.push_str("\"></a>");
        }
        out.push_str("<div class=\"meta\"><span>");
        push_escaped(out, source_label(first));
        out.push_str("</span><span>ev ");
        push_escaped(out, &prefix(&first.id().0));
        out.push_str("</span></div>\n<p class=\"excerpt\">");
        push_escaped(out, first.excerpt());
        out.push_str("</p>\n<div class=\"foot\"><span>fetched by ");
        push_escaped(out, &fetched_by(page, group));
        out.push_str("</span><span>");
        out.push_str(&cited_phrase(group.citations));
        out.push_str("</span></div></article>\n");
    }
    if total > FACTS_SHOWN {
        out.push_str("<a href=\"days/");
        push_escaped(out, &page.day.to_string());
        out.push_str(&format!(".html\">All {total} facts on the day page</a>\n"));
    }
    out.push_str("</section>\n");
}

struct FactGroup<'a> {
    records: Vec<&'a Evidence>,
    citations: usize,
}

fn fact_groups<'a>(page: &DayPage<'a>) -> Vec<FactGroup<'a>> {
    let cited = citations(
        page.arguments
            .iter()
            .map(|argument| argument.cites.iter().map(|evidence| evidence.id())),
    );
    let mut by_digest: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
    for record in page.evidence {
        by_digest.entry(record.sha256()).or_default().push(record);
    }
    let mut groups: Vec<FactGroup<'_>> = by_digest
        .into_values()
        .map(|mut records| {
            records.sort_by(|left, right| {
                left.fetched_at_ms()
                    .cmp(&right.fetched_at_ms())
                    .then(left.id().0.cmp(&right.id().0))
            });
            let citations = records
                .iter()
                .map(|record| cited.get(&record.id().0).copied().unwrap_or(0))
                .sum();
            FactGroup { records, citations }
        })
        .collect();
    groups.sort_by(|left, right| {
        right
            .citations
            .cmp(&left.citations)
            .then_with(|| {
                left.records[0]
                    .fetched_at_ms()
                    .cmp(&right.records[0].fetched_at_ms())
            })
            .then_with(|| left.records[0].id().0.cmp(&right.records[0].id().0))
    });
    groups
}

fn fetched_by(page: &DayPage<'_>, group: &FactGroup<'_>) -> String {
    let mut names = Vec::new();
    for record in &group.records {
        let name = display_name(&page.chrome, record.author());
        if !names.iter().any(|seen: &String| seen == &name) {
            names.push(name);
        }
    }
    names.join(", ")
}

fn cited_phrase(count: usize) -> String {
    match count {
        0 => "not cited".to_owned(),
        1 => "cited once".to_owned(),
        n => format!("cited {n} times"),
    }
}

fn source_label(evidence: &Evidence) -> &str {
    match evidence.url().host_str() {
        Some(host) if host == WIKIMEDIA_HOST => "Wikimedia · On this day",
        Some(host) if host == SUNRISE_HOST => "sunrise-sunset.org · London",
        Some(host) => host,
        None => "source",
    }
}

enum Event<'a> {
    Argument {
        at_ms: u64,
        author: &'a str,
        id: &'a str,
        text: &'a str,
        cites: &'a [Evidence],
    },
    Change {
        at_ms: u64,
        author: String,
        from: Colour,
        to: Colour,
    },
    Marker {
        at_ms: u64,
        colour: Colour,
        count: usize,
    },
}

impl Event<'_> {
    fn at_ms(&self) -> u64 {
        match self {
            Self::Argument { at_ms, .. }
            | Self::Change { at_ms, .. }
            | Self::Marker { at_ms, .. } => *at_ms,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Argument { .. } => 0,
            Self::Change { .. } => 1,
            Self::Marker { .. } => 2,
        }
    }

    fn tie(&self) -> &str {
        match self {
            Self::Argument { id, .. } => id,
            Self::Change { author, .. } => author,
            Self::Marker { .. } => "",
        }
    }
}

fn timeline<'a>(page: &'a DayPage<'a>, live: &[String], now: u64, open: u64) -> Vec<Event<'a>> {
    let mut events = Vec::new();
    for argument in page.arguments {
        if (open..=now).contains(&argument.at_ms) {
            events.push(Event::Argument {
                at_ms: argument.at_ms,
                author: &argument.author,
                id: &argument.id,
                text: &argument.text,
                cites: &argument.cites,
            });
        }
    }
    for (author, trail) in trails(page.votes) {
        for pair in trail.windows(2) {
            let (at_ms, to) = pair[1];
            let from = pair[0].1;
            if from != to && (open..=now).contains(&at_ms) {
                events.push(Event::Change {
                    at_ms,
                    author: author.clone(),
                    from,
                    to,
                });
            }
        }
    }
    if let Some(hold) = holding(page.votes, live, open, now) {
        let current = current_votes(page.votes, hold.from_ms);
        let count = live
            .iter()
            .filter(|author| current.get(author.as_str()) == Some(&hold.colour))
            .count();
        events.push(Event::Marker {
            at_ms: hold.from_ms,
            colour: hold.colour,
            count,
        });
    }
    events.sort_by(|left, right| {
        right
            .at_ms()
            .cmp(&left.at_ms())
            .then(left.rank().cmp(&right.rank()))
            .then_with(|| left.tie().cmp(right.tie()))
    });
    events.truncate(8);
    events
}

fn display_name(chrome: &super::Chrome<'_>, id: &str) -> String {
    chrome
        .names
        .get(id)
        .cloned()
        .unwrap_or_else(|| id.to_owned())
}

fn prefix(id: &str) -> String {
    id.chars().take(8).collect()
}

fn hhmm(unix_ms: u64) -> String {
    let shown = utc_time(unix_ms);
    shown[..5].to_owned()
}

fn hex(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!("#{red:02X}{green:02X}{blue:02X}")
}

/// One decimal, half up. `37 / 60` is `61.7%`.
fn width_percent(capped_ms: u64) -> String {
    let tenths = (capped_ms.saturating_mul(1000) + HOLD_MS / 2) / HOLD_MS;
    format!("{}.{}%", tenths / 10, tenths % 10)
}

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use url::Url;

    use super::render_front;
    use crate::closing::Vote;
    use crate::colour::Colour::{Black, Blue, Green, Red, Yellow};
    use crate::day::Day;
    use crate::evidence::Evidence;
    use crate::site::support::{
        FORBIDDEN, assert_clean, body_open, evidence, page, page_named, vote,
    };
    use crate::site::{ArgumentView, Chrome, DayPage, Paint};
    use crate::tally::{vote_changes, votes_cast};

    fn at(hour: u64, minute: u64) -> u64 {
        hour * 3_600_000 + minute * 60_000
    }

    fn names(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, name)| ((*id).to_owned(), (*name).to_owned()))
            .collect()
    }

    fn page_at<'a>(
        votes: &'a [Vote],
        arguments: &'a [ArgumentView],
        evidence: &'a [Evidence],
        painted_by: Paint,
        as_of_ms: u64,
        names: &'a BTreeMap<String, String>,
    ) -> DayPage<'a> {
        DayPage {
            day: Day::from_ms(as_of_ms),
            votes,
            arguments,
            evidence,
            decision: None,
            chrome: Chrome {
                painted_by,
                as_of_ms,
                rendered_at_ms: 3_723_000,
                server: "peer-a",
                names,
            },
        }
    }

    fn sourced(author: &str, url: &str, at_ms: u64, text: &str) -> Evidence {
        Evidence::new(
            author.to_owned(),
            "run".to_owned(),
            Url::parse(url).unwrap(),
            at_ms,
            text,
        )
    }

    fn argument(
        id: &str,
        author: &str,
        at_ms: u64,
        text: &str,
        cites: Vec<Evidence>,
    ) -> ArgumentView {
        ArgumentView {
            id: id.to_owned(),
            author: author.to_owned(),
            at_ms,
            text: text.to_owned(),
            cites,
        }
    }

    /// `assert_clean` also pins the shell fixture clock at 00:00:00.
    /// A page read at another time keeps the same prohibitions.
    fn assert_prohibitions(html: &str) {
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"), "{html}");
        assert!(!lower.contains("<link"), "{html}");
        assert!(!lower.contains("src="), "{html}");
        for word in FORBIDDEN {
            assert!(!lower.contains(word), "{word} in {html}");
        }
    }

    #[test]
    fn render_front_paints_black_with_white_text_and_yellow_with_black_text() {
        let black = page(&[], &[], &[], None, Paint::Decided(Black));
        let html = render_front(&black);
        assert!(html.contains("<body style=\"background:#000000;color:#FFFFFF\">"));
        assert!(html.contains("background:#000000"));
        assert!(html.contains("color:#FFFFFF"));
        assert_clean(&html);

        let yellow = page(&[], &[], &[], None, Paint::Decided(Yellow));
        let html = render_front(&yellow);
        assert!(html.contains("<body style=\"background:#FFFF00;color:#000000\">"));
        assert!(html.contains("background:#FFFF00"));
        assert!(html.contains("color:#000000"));
        assert!(!html.contains("color:#FFFFFF"));
    }

    #[test]
    fn render_front_undecided_and_no_record_paint_nothing() {
        let undecided = page(&[], &[], &[], None, Paint::Undecided);
        let html = render_front(&undecided);
        assert!(html.contains("Yesterday was undecided under the rule"));
        assert_eq!(body_open(&html), "<body>");
        assert_clean(&html);

        let missing = page(&[], &[], &[], None, Paint::NoRecord);
        let html = render_front(&missing);
        assert!(html.contains("No result is recorded for yesterday"));
        assert_eq!(body_open(&html), "<body>");
    }

    #[test]
    fn named_peer_is_shown_beside_the_id_and_escaped() {
        let mut peer_names = BTreeMap::new();
        peer_names.insert("peer-a".to_owned(), "<Wren&>".to_owned());
        let arguments = [
            argument("arg-1", "peer-a", 0, "named", Vec::new()),
            argument("arg-2", "ada", 0, "unnamed", Vec::new()),
        ];
        let shown = page_named(
            &[],
            &arguments,
            &[],
            None,
            Paint::NoRecord,
            "peer-a",
            &peer_names,
        );
        let html = render_front(&shown);
        assert!(html.contains("&lt;Wren&amp;&gt; <code>peer-a</code>"));
        assert!(!html.contains("<Wren&>"));
        assert!(html.contains("<code>ada</code>"));
        assert!(html.contains(
            "Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC by &lt;Wren&amp;&gt; (peer-a)"
        ));
        assert_clean(&html);
    }

    #[test]
    fn paint_sentences_name_the_three_records() {
        let decided = render_front(&page(&[], &[], &[], None, Paint::Decided(Blue)));
        assert!(decided.contains("This page is blue because yesterday the swarm decided blue."));
        assert_clean(&decided);

        let undecided = render_front(&page(&[], &[], &[], None, Paint::Undecided));
        assert!(
            undecided.contains(
                "Yesterday was undecided under the rule, so today this page is unpainted."
            )
        );
        assert_clean(&undecided);

        let missing = render_front(&page(&[], &[], &[], None, Paint::NoRecord));
        assert!(
            missing
                .contains("No result is recorded for yesterday, so today this page is unpainted.")
        );
        assert_clean(&missing);
    }

    #[test]
    fn one_live_voter_uses_the_swarm_heading() {
        let votes = [vote("ada", Red, 0)];
        let html = render_front(&page(&votes, &[], &[], None, Paint::NoRecord));
        assert!(html.contains("The swarm is choosing tomorrow\u{2019}s colour."));
        assert!(!html.contains("small models are choosing"));
        assert_clean(&html);
    }

    #[test]
    fn seven_of_twelve_hold_red_through_15_42() {
        let mut votes = Vec::new();
        for index in 0..7 {
            votes.push(vote(&format!("red-{index}"), Red, at(15, 5)));
        }
        for index in 0..5 {
            votes.push(vote(&format!("blue-{index}"), Blue, at(14, 0)));
        }
        let names = BTreeMap::new();
        let shown = page_at(&votes, &[], &[], Paint::NoRecord, at(15, 42), &names);
        let html = render_front(&shown);
        assert!(html.contains("12 small models are choosing tomorrow\u{2019}s colour."));
        assert!(html.contains("THURSDAY 1 JANUARY 1970 · OPEN 00:00–23:00 UTC"));
        assert!(html.contains("RIGHT NOW · 15:42 UTC"));
        assert!(html.contains("7 of 12 live voters · more than half needs 7"));
        assert!(html.contains("held since 15:05"));
        assert!(html.contains("37 of 60 min"));
        assert!(html.contains("width:61.7%"));
        assert!(html.contains(
            "If no vote moves before <strong>16:05 UTC</strong>, red is the day\u{2019}s colour"
        ));
        assert!(html.contains("red now holds 7 of 12, more than half. The hour starts."));
        assert!(html.contains(
            "Records as of 15:42:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC"
        ));
        let hero = html.split_once("<section class=\"hero\">").unwrap().1;
        let hero = hero.split_once("aria-labelledby=\"votes-h\"").unwrap().0;
        assert!(hero.starts_with("<div class=\"hero-text\">"));
        assert!(hero.contains("<section class=\"card hero-card\">"));
        assert!(hero.ends_with("</section>\n</section>\n<section class=\"card\" "));
        assert!(hero.contains(
            "<span class=\"swatch\" aria-hidden=\"true\" style=\"background:#FF0000\"></span>"
        ));
        assert!(hero.contains("<span class=\"big\">Red</span>"));
        assert!(hero.contains("class=\"fill\" style=\"width:61.7%;background:#FF0000\""));
        assert_eq!(hero.matches("class=\"tick\"").count(), 3);
        for tick in [25, 50, 75] {
            assert!(hero.contains(&format!("class=\"tick\" style=\"left:{tick}%\"")));
        }
        assert!(hero.contains("<div class=\"bar-labels mono\"><span>held since 15:05</span><span>37 of 60 min</span><span>16:05</span></div>"));
        assert!(html.contains("<div class=\"mark\">red now holds 7 of 12"));
        assert_prohibitions(&html);
    }

    #[test]
    fn decided_blue_and_a_day_with_no_votes() {
        let votes = [
            vote("a", Blue, at(14, 0)),
            vote("b", Blue, at(14, 0)),
            vote("c", Blue, at(14, 0)),
        ];
        let names = BTreeMap::new();
        let decided = render_front(&page_at(
            &votes,
            &[],
            &[],
            Paint::NoRecord,
            at(15, 0),
            &names,
        ));
        assert!(decided.contains("<span class=\"big\">Blue</span>, held 14:00–15:00 UTC"));
        assert!(!decided.contains("held since"));
        assert!(!decided.contains("of 60 min"));
        assert_prohibitions(&decided);

        let empty = render_front(&page(&[], &[], &[], None, Paint::NoRecord));
        assert!(empty.contains("No votes yet today."));
        assert_clean(&empty);
    }

    #[test]
    fn tiles_group_by_count_then_colour_and_show_names() {
        let votes = [
            vote("a", Red, 0),
            vote("b", Red, 0),
            vote("c", Green, 0),
            vote("d", Blue, 0),
        ];
        let peer_names = names(&[("a", "Wren"), ("b", "Avocet"), ("c", "Ibis"), ("d", "Lark")]);
        let html = render_front(&page_named(
            &votes,
            &[],
            &[],
            None,
            Paint::NoRecord,
            "peer-a",
            &peer_names,
        ));
        let red = html.find("></span>red 2</span>").expect("red chip");
        let green = html.find("></span>green 1</span>").expect("green chip");
        let blue = html.find("></span>blue 1</span>").expect("blue chip");
        assert!(red < green && green < blue);
        let avocet = html.find("Avocet").expect("avocet");
        let wren = html.find("Wren").expect("wren");
        let ibis = html.find("Ibis").expect("ibis");
        let lark = html.find("Lark").expect("lark");
        assert!(avocet < wren && wren < ibis && ibis < lark);
        assert!(html.contains("href=\"flock.html\""));
        let current = html.split_once("aria-labelledby=\"votes-h\"").unwrap().1;
        let current = current.split_once("</section>").unwrap().0;
        assert_eq!(current.matches("class=\"chip-lg\"").count(), 3);
        assert_eq!(current.matches("class=\"swatch-sm\"").count(), 3);
        assert!(current.contains("<ol class=\"tiles\">"));
        assert_eq!(current.matches("<li class=\"tile\">").count(), 4);
        assert_eq!(current.matches("class=\"paint mono\"").count(), 4);
        assert_eq!(current.matches("class=\"name\"").count(), 4);
        assert_eq!(current.matches("class=\"id mono\"").count(), 4);
        assert!(
            current.contains("<span class=\"name\">Avocet</span><span class=\"id mono\">b</span>")
        );
        assert_eq!(
            current
                .matches("<summary>How these are counted</summary>")
                .count(),
            1
        );
        assert!(!current.contains("<summary>formula</summary>"));
        assert_eq!(current.matches("<p class=\"mono\">").count(), 2);
        assert_clean(&html);
    }

    #[test]
    fn five_numbers_match_the_tally_functions_beside_their_rules() {
        let votes = [
            vote("ada", Red, 0),
            vote("ada", Blue, 0),
            vote("bea", Green, 0),
        ];
        let arguments = [
            argument("a1", "ada", 0, "one", Vec::new()),
            argument("a2", "ada", 0, "two", Vec::new()),
            argument("a3", "bea", 0, "three", Vec::new()),
            argument("a4", "bea", 0, "four", Vec::new()),
            argument("a5", "bea", 0, "five", Vec::new()),
        ];
        let records = [
            evidence("f0"),
            evidence("f1"),
            evidence("f2"),
            evidence("f3"),
        ];
        let shown = page(&votes, &arguments, &records, None, Paint::NoRecord);
        let html = render_front(&shown);
        let live = 2;
        let cast = votes_cast(&votes, shown.day.open_ms(), shown.chrome.as_of_ms);
        let changes: usize = vote_changes(&votes).values().sum();
        assert_eq!(cast, 3);
        assert_eq!(changes, 1);
        assert_eq!((live, cast, changes), (2, 3, 1));
        assert_beside(
            &html,
            "<span class=\"n\">2</span><span class=\"label\">live voters</span>",
            "The live voters are the distinct authors",
        );
        assert_beside(
            &html,
            "<span class=\"n\">3</span><span class=\"label\">votes cast</span>",
            "Votes cast is the number of vote records",
        );
        assert_beside(
            &html,
            "<span class=\"n\">1</span><span class=\"label\">vote changes</span>",
            "A vote change is a vote whose colour differs",
        );
        assert_beside(
            &html,
            "<span class=\"n\">4</span><span class=\"label\">fetches</span>",
            "fetch count is the number of evidence records that author stored",
        );
        assert_beside(
            &html,
            "<span class=\"n\">5</span><span class=\"label\">arguments</span>",
            "An argument count is the number of argue records",
        );
        let numbers = html
            .split_once("<section class=\"numbers\" aria-label=\"Today in numbers\">")
            .unwrap()
            .1;
        let numbers = numbers.split_once("</section>").unwrap().0;
        assert_eq!(
            numbers
                .matches("<div class=\"card\"><span class=\"n\">")
                .count(),
            5
        );
        assert_eq!(numbers.matches("</span><span class=\"label\">").count(), 5);
        assert_eq!(numbers.matches("<summary>formula</summary>").count(), 5);
        assert_clean(&html);
    }

    fn assert_beside(html: &str, label: &str, rule: &str) {
        let start = html
            .find(label)
            .unwrap_or_else(|| panic!("missing {label}"));
        let after = start + label.len();
        let rule_at = html[after..]
            .find(rule)
            .unwrap_or_else(|| panic!("missing {rule} after {label}"));
        let next = html[after..]
            .find("<div class=\"card\">")
            .unwrap_or(usize::MAX);
        assert!(rule_at < next, "{rule} is not beside {label}");
    }

    #[test]
    fn day_so_far_keeps_eight_newest_and_the_hold_marker_links_a_fact() {
        let names = BTreeMap::new();
        let mut arguments = Vec::new();
        for index in 0..9 {
            arguments.push(argument(
                &format!("arg-{index}"),
                "ada",
                at(0, index),
                &format!("slot-{index:02}"),
                Vec::new(),
            ));
        }
        let capped = render_front(&page_at(
            &[],
            &arguments,
            &[],
            Paint::NoRecord,
            at(0, 8),
            &names,
        ));
        assert!(capped.find("slot-08").unwrap() < capped.find("slot-01").unwrap());
        assert!(!capped.contains("slot-00"));
        assert_prohibitions(&capped);

        let cited = sourced(
            "ada",
            "https://api.wikimedia.org/on-this-day",
            at(10, 0),
            "cited-excerpt",
        );
        let fact_id = cited.id().0.clone();
        let votes = [vote("ada", Red, at(10, 0)), vote("ada", Blue, at(11, 0))];
        let argued = [argument(
            "arg-late",
            "ada",
            at(12, 0),
            "later-argument",
            vec![cited.clone()],
        )];
        let records = [cited];
        let html = render_front(&page_at(
            &votes,
            &argued,
            &records,
            Paint::NoRecord,
            at(13, 0),
            &names,
        ));
        assert!(html.find("later-argument").unwrap() < html.find("changed vote").unwrap());
        assert!(html.contains("→"));
        assert!(html.contains(&format!("href=\"#fact-{fact_id}\"")));
        assert!(html.contains(&format!("id=\"fact-{fact_id}\"")));
        let thread = html.split_once("<ol class=\"timeline\">").unwrap().1;
        let thread = thread.split_once("</ol>").unwrap().0;
        assert_eq!(thread.matches("<span class=\"t mono\">").count(), 3);
        assert_eq!(
            thread
                .matches("class=\"rail\" aria-hidden=\"true\"")
                .count(),
            3
        );
        assert_eq!(
            thread
                .matches("class=\"dot\" style=\"background:#0000FF\"")
                .count(),
            3
        );
        assert_eq!(thread.matches("<span class=\"line\"></span>").count(), 3);
        assert_eq!(thread.matches("<div class=\"body\">").count(), 3);
        assert!(thread.contains("<blockquote>“later-argument”</blockquote>"));
        assert!(thread.contains(&format!(
            "<a class=\"cite\" href=\"#fact-{fact_id}\">cites {}",
            &fact_id[..8]
        )));
        assert!(thread.contains("<span aria-hidden=\"true\">→</span>"));
        assert!(thread.contains("<div class=\"mark\">"));
        assert!(capped.contains("class=\"dot\" style=\"background:var(--chip)\""));
        assert_prohibitions(&html);
    }

    #[test]
    fn fact_cards_keep_excerpt_lines_and_escape_record_and_tile_text() {
        let record = sourced(
            "peer<&",
            "https://api.wikimedia.org/on-this-day",
            0,
            "first <line>\nsecond & line",
        );
        let id = record.id().0.clone();
        let records = [record];
        let votes = [vote("peer<&", Red, 0), vote("other", Blue, 0)];
        let peer_names = names(&[("peer<&", "<Wren&>")]);
        let html = render_front(&page_at(
            &votes,
            &[],
            &records,
            Paint::NoRecord,
            0,
            &peer_names,
        ));
        assert!(html.contains(&format!("<article class=\"card fact\" id=\"fact-{id}\">")));
        assert!(html.contains(&format!(
            "<div class=\"meta\"><span>Wikimedia · On this day</span><span>ev {}</span></div>",
            &id[..8]
        )));
        assert!(html.contains("<p class=\"excerpt\">first &lt;line&gt;\nsecond &amp; line</p>"));
        assert!(html.contains("<div class=\"foot\"><span>fetched by &lt;Wren&amp;&gt;</span><span>not cited</span></div>"));
        assert!(html.contains("<span class=\"name\">&lt;Wren&amp;&gt;</span><span class=\"id mono\">peer&lt;&amp;</span>"));
        let right_now = html
            .split_once("<section class=\"card hero-card\">")
            .unwrap()
            .1;
        let right_now = right_now.split_once("</section>").unwrap().0;
        assert!(right_now.contains("No colour has more than half of the 2 live voters."));
        assert_eq!(right_now.matches("class=\"swatch\"").count(), 2);
        assert!(right_now.contains("<span class=\"big\">Red</span>"));
        assert!(right_now.contains("<span class=\"big\">Blue</span>"));
        assert_clean(&html);
    }

    #[test]
    fn one_digest_is_one_card_and_thirteen_facts_link_the_rest() {
        let peer_names = names(&[("ada", "Wren"), ("bea", "Robin")]);
        let left = sourced(
            "ada",
            "https://api.wikimedia.org/on-this-day",
            10,
            "shared-excerpt",
        );
        let right = sourced(
            "bea",
            "https://api.sunrise-sunset.org/json",
            20,
            "shared-excerpt",
        );
        let arguments = [
            argument("c1", "ada", 0, "cites left", vec![left.clone()]),
            argument("c2", "bea", 0, "cites right", vec![right.clone()]),
        ];
        let paired = [left, right];
        let html = render_front(&page_named(
            &[],
            &arguments,
            &paired,
            None,
            Paint::NoRecord,
            "peer-a",
            &peer_names,
        ));
        assert_eq!(html.matches("shared-excerpt").count(), 1);
        assert!(html.contains("fetched by Wren, Robin"));
        assert!(html.contains("cited 2 times"));
        assert!(html.contains("Wikimedia · On this day"));
        assert_eq!(html.matches("id=\"fact-").count(), 2);
        assert!(!html.contains("All 2 facts"));
        assert_clean(&html);

        let mut many = Vec::new();
        for index in 0..13 {
            let url = if index % 2 == 0 {
                "https://api.wikimedia.org/on-this-day"
            } else {
                "https://api.sunrise-sunset.org/json"
            };
            many.push(sourced(
                "ada",
                url,
                index,
                &format!("distinct-excerpt-{index:02}"),
            ));
        }
        let empty_names = BTreeMap::new();
        let broad = render_front(&page_at(&[], &[], &many, Paint::NoRecord, 0, &empty_names));
        for index in 0..12 {
            assert!(broad.contains(&format!("distinct-excerpt-{index:02}")));
        }
        assert!(!broad.contains("distinct-excerpt-12"));
        assert!(broad.contains("All 13 facts"));
        assert!(broad.contains("href=\"days/1970-01-01.html\""));
        assert!(broad.contains("sunrise-sunset.org · London"));
        assert!(broad.contains("not cited"));
        assert_clean(&broad);
    }
}
