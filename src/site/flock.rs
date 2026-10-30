//! The flock page: who is voting what, over one day's records.

use std::collections::{BTreeMap, BTreeSet};

use crate::colour::Colour;
use crate::day::utc_time;
use crate::tally::{
    ARGUMENTS_RULE, CURRENT_VOTES_RULE, FETCHES_RULE, TRAIL_RULE, VOTE_CHANGES_RULE, current_votes,
    fetches, trails, vote_changes,
};

use super::layout::{card, chip, document, push_escaped};
use super::{DayPage, Section};

const HERO_TAIL: &str = " each a small model on its own machine. None is given a personality. \
     They differ only in which facts they could afford to read, and what they did with them.";

const NAMES_NOTE: &str = "Names are drawn from a hash of the peer id and are only for reading. \
     The peer id is the identity.";

const GROUP_NOTE: &str = "A group here is only the peers whose newest vote names the same \
     colour. It says nothing about why.";

struct Peer {
    id: String,
    display: String,
    trail: Vec<Colour>,
    changes: usize,
    fetches: usize,
    arguments: usize,
    last_ms: u64,
}

/// The flock, at `page.chrome.as_of_ms`. Depth 0, section [`Section::Flock`].
pub fn render_flock(page: &DayPage<'_>) -> String {
    let rows = peer_rows(page);
    let mut body = String::from(
        "<section class=\"hero\"><div class=\"hero-text\"><h1>The flock</h1>\n<p class=\"lede\">",
    );
    body.push_str(&hero(rows.len()));
    body.push_str("</p></div></section>\n");
    body.push_str(&card(
        &format!(
            "Grouped by current vote · {} UTC",
            hhmm(page.chrome.as_of_ms)
        ),
        &groups(page),
    ));
    let mut table = String::new();
    push_table(&mut table, page, &rows);
    table.push_str(&counting_notes());
    body.push_str(&card("Every peer today", &table));
    document("The flock", Some(Section::Flock), 0, &page.chrome, &body)
}

fn hero(count: usize) -> String {
    match count {
        0 => "No peer has written anything today.".to_owned(),
        1 => "One peer, a small model on its own machine, given no personality.".to_owned(),
        n => format!("{n} peers,{HERO_TAIL}"),
    }
}

/// `HH:MM` from [`utc_time`]'s `HH:MM:SS`.
fn hhmm(unix_ms: u64) -> String {
    let full = utc_time(unix_ms);
    match full.rsplit_once(':') {
        Some((hours_minutes, _)) => hours_minutes.to_owned(),
        None => full,
    }
}

fn display_name(page: &DayPage<'_>, id: &str) -> String {
    page.chrome
        .names
        .get(id)
        .cloned()
        .unwrap_or_else(|| id.to_owned())
}

fn note_last<'a>(last_at: &mut BTreeMap<&'a str, u64>, id: &'a str, at_ms: u64) {
    last_at
        .entry(id)
        .and_modify(|have| *have = (*have).max(at_ms))
        .or_insert(at_ms);
}

fn peer_rows(page: &DayPage<'_>) -> Vec<Peer> {
    let mut ids = BTreeSet::new();
    for vote in page.votes {
        ids.insert(vote.author.as_str());
    }
    for argument in page.arguments {
        ids.insert(argument.author.as_str());
    }
    for record in page.evidence {
        ids.insert(record.author());
    }

    let vote_trails = trails(page.votes);
    let changes = vote_changes(page.votes);
    let fetch_counts = fetches(page.evidence);
    let mut argument_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut last_at: BTreeMap<&str, u64> = BTreeMap::new();
    for argument in page.arguments {
        *argument_counts.entry(argument.author.as_str()).or_default() += 1;
        note_last(&mut last_at, argument.author.as_str(), argument.at_ms);
    }
    for vote in page.votes {
        note_last(&mut last_at, vote.author.as_str(), vote.at_ms);
    }
    for record in page.evidence {
        note_last(&mut last_at, record.author(), record.fetched_at_ms());
    }

    let mut rows: Vec<Peer> = ids
        .into_iter()
        .map(|id| Peer {
            display: display_name(page, id),
            trail: vote_trails
                .get(id)
                .map(|votes| votes.iter().map(|(_, colour)| *colour).collect())
                .unwrap_or_default(),
            changes: changes.get(id).copied().unwrap_or(0),
            fetches: fetch_counts.get(id).copied().unwrap_or(0),
            arguments: argument_counts.get(id).copied().unwrap_or(0),
            last_ms: *last_at.get(id).expect("peer has a record"),
            id: id.to_owned(),
        })
        .collect();
    rows.sort_by(|left, right| {
        left.display
            .cmp(&right.display)
            .then(left.id.cmp(&right.id))
    });
    rows
}

fn groups(page: &DayPage<'_>) -> String {
    let mut body = String::new();
    push_columns(&mut body, page);
    body.push_str("<p class=\"muted\" style=\"font-size:14px\">");
    body.push_str(GROUP_NOTE);
    body.push_str("</p>\n");
    // Fixed rule text has no markup. Inserting it raw keeps the constant on
    // the page; `formula` would escape its apostrophes.
    body.push_str("<details class=\"formula\"><summary>formula</summary><p class=\"mono\">");
    body.push_str(CURRENT_VOTES_RULE);
    body.push_str("</p></details>");
    body
}

fn push_columns(out: &mut String, page: &DayPage<'_>) {
    let current = current_votes(page.votes, page.chrome.as_of_ms);
    let mut grouped: BTreeMap<Colour, Vec<(String, String)>> = BTreeMap::new();
    for (author, colour) in current {
        grouped
            .entry(colour)
            .or_default()
            .push((display_name(page, &author), author));
    }
    let mut columns: Vec<(Colour, Vec<(String, String)>)> = grouped.into_iter().collect();
    for (_, members) in &mut columns {
        members.sort();
    }
    columns.sort_by(|left, right| right.1.len().cmp(&left.1.len()).then(left.0.cmp(&right.0)));
    if columns.is_empty() {
        return;
    }
    out.push_str("<div style=\"display:flex;flex-wrap:wrap;gap:8px\">\n");
    for (colour, members) in columns {
        let count = members.len();
        let rgb = colour.srgb();
        let ink = contrasting_text(rgb);
        let [red, green, blue] = rgb;
        out.push_str(&format!(
            "<div style=\"flex:{count} 1 150px;min-width:0;border-radius:18px;overflow:hidden;box-shadow:inset 0 0 0 1px rgba(0,0,0,0.1);display:flex;flex-direction:column\">\n<div style=\"background:#{red:02X}{green:02X}{blue:02X};color:{ink};padding:16px 18px;display:flex;justify-content:space-between;align-items:baseline;gap:8px\">"
        ));
        out.push_str("<span style=\"font-weight:800;font-size:24px;letter-spacing:-0.02em\">");
        push_escaped(out, colour.name());
        out.push_str("</span><span style=\"font-weight:800;font-size:34px;line-height:1\">");
        out.push_str(&count.to_string());
        out.push_str("</span></div>\n<ul style=\"list-style:none;margin:0;padding:12px 18px 16px;display:flex;flex-wrap:wrap;gap:6px 14px;font-size:15px;font-weight:600\">\n");
        for (name, _) in &members {
            out.push_str("<li>");
            push_escaped(out, name);
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n</div>\n");
    }
    out.push_str("</div>\n");
}

fn push_table(out: &mut String, page: &DayPage<'_>, rows: &[Peer]) {
    out.push_str(
        "<div style=\"overflow-x:auto\">\n<table style=\"min-width:760px\">\n<thead>\n<tr>",
    );
    for (index, label) in [
        "PEER",
        "VOTES TODAY, OLDEST FIRST",
        "CHANGES",
        "FETCHES",
        "ARGUMENTS",
        "LAST ACTIVE",
    ]
    .into_iter()
    .enumerate()
    {
        out.push_str(if index >= 2 {
            "<th scope=\"col\" class=\"num\">"
        } else {
            "<th scope=\"col\">"
        });
        out.push_str(label);
        out.push_str("</th>");
    }
    out.push_str("</tr>\n</thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>\n<td>");
        out.push_str("<a href=\"flock.html\" style=\"text-decoration:none;display:flex;flex-direction:column;line-height:1.25\">");
        if let Some(name) = page.chrome.names.get(&row.id) {
            out.push_str("<span style=\"font-weight:700\">");
            push_escaped(out, name);
            out.push_str("</span>");
        }
        out.push_str("<span class=\"mono muted\" style=\"font-size:12px\">");
        push_escaped(out, &row.id);
        out.push_str("</span></a>");
        out.push_str("</td>\n<td>");
        if row.trail.is_empty() {
            out.push_str("no vote yet");
        } else {
            out.push_str("<span class=\"chips\" style=\"gap:4px\">");
            for colour in &row.trail {
                out.push_str(&chip(*colour));
            }
            out.push_str("</span>");
        }
        out.push_str("</td>\n<td class=\"num\">");
        out.push_str(&row.changes.to_string());
        out.push_str("</td>\n<td class=\"num\">");
        out.push_str(&row.fetches.to_string());
        out.push_str("</td>\n<td class=\"num\">");
        out.push_str(&row.arguments.to_string());
        out.push_str("</td>\n<td class=\"num\">");
        out.push_str(&hhmm(row.last_ms));
        out.push_str("</td>\n</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</div>\n");
}

fn counting_notes() -> String {
    let mut html = String::from("<details>\n<summary>How each column is counted</summary>\n");
    for rule in [VOTE_CHANGES_RULE, TRAIL_RULE, FETCHES_RULE, ARGUMENTS_RULE] {
        html.push_str("<p class=\"mono\">");
        html.push_str(rule);
        html.push_str("</p>\n");
    }
    html.push_str("<p>");
    html.push_str(NAMES_NOTE);
    html.push_str("</p>\n</details>\n");
    html
}

/// Black or white, whichever has the higher WCAG contrast against `rgb`.
///
/// The same choice as the shell. That function is private to `layout`, and
/// this page may not add a file to export it.
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

    use super::render_flock;
    use crate::colour::Colour::{Blue, Green, Navy, Red};
    use crate::day::{Day, utc_time};
    use crate::evidence::Evidence;
    use crate::site::layout::chip;
    use crate::site::support::{self, FORBIDDEN, assert_clean, page, page_named};
    use crate::site::{ArgumentView, Chrome, DayPage, Paint};
    use crate::tally::{
        ARGUMENTS_RULE, CURRENT_VOTES_RULE, FETCHES_RULE, TRAIL_RULE, VOTE_CHANGES_RULE,
    };

    const MINUTE_MS: u64 = 60_000;

    fn assert_rules(html: &str) {
        for rule in [
            CURRENT_VOTES_RULE,
            VOTE_CHANGES_RULE,
            TRAIL_RULE,
            FETCHES_RULE,
            ARGUMENTS_RULE,
        ] {
            assert!(html.contains(rule), "missing {rule}");
        }
        assert!(html.contains(super::GROUP_NOTE));
        assert!(html.contains(super::NAMES_NOTE));
        assert!(html.contains("<summary>formula</summary>"));
        assert!(html.contains("<summary>How each column is counted</summary>"));
    }

    fn assert_marked_clean(html: &str) {
        let lower = html.to_ascii_lowercase();
        assert!(!lower.contains("<script"), "{html}");
        assert!(!lower.contains("<link"));
        assert!(!lower.contains("src="));
        for word in FORBIDDEN {
            assert!(!lower.contains(word), "{word} in {html}");
        }
    }

    fn names(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, name)| ((*id).to_owned(), (*name).to_owned()))
            .collect()
    }

    #[test]
    fn no_peer_states_that_nothing_was_written_and_still_publishes_the_rules() {
        let html = render_flock(&page(&[], &[], &[], None, Paint::NoRecord));
        assert!(html.contains("<h1>The flock</h1>"));
        assert!(html.contains("No peer has written anything today."));
        assert!(html.contains("Grouped by current vote · 00:00 UTC"));
        assert!(!html.contains("flex:1 1 150px"));
        assert!(html.contains("href=\"flock.html\" aria-current=\"page\""));
        assert!(!html.contains("<li><a"));
        assert!(!html.contains("<td><a"));
        assert_rules(&html);
        assert_clean(&html);
    }

    #[test]
    fn one_peer_uses_the_singular_hero_and_escapes_the_name() {
        let mut labels = BTreeMap::new();
        labels.insert("peer-a".to_owned(), "<Wren&>".to_owned());
        let votes = [support::vote("peer-a", Red, 0)];
        let shown = page_named(&votes, &[], &[], None, Paint::NoRecord, "peer-a", &labels);
        let html = render_flock(&shown);
        assert!(html.contains(
            "<section class=\"hero\"><div class=\"hero-text\"><h1>The flock</h1>\n<p class=\"lede\">One peer, a small model on its own machine, given no personality.</p></div></section>"
        ));
        assert!(!html.contains("One peer, each"));
        assert!(!html.contains("1 peers"));
        assert!(html.contains("<li>&lt;Wren&amp;&gt;</li>"));
        assert!(html.contains(
            "<span style=\"font-weight:700\">&lt;Wren&amp;&gt;</span><span class=\"mono muted\" style=\"font-size:12px\">peer-a</span></a>"
        ));
        assert!(!html.contains("<Wren&>"));
        assert!(html.contains("flex:1 1 150px;"));
        assert!(html.contains("background:#FF0000;color:#000000"));
        assert_rules(&html);
        assert_clean(&html);
    }

    #[test]
    fn twelve_peers_group_by_count_then_colour_in_padded_tiles() {
        let roster = [
            ("r7", "Avocet", Red),
            ("r6", "Bunting", Red),
            ("r5", "Crane", Red),
            ("r4", "Dunlin", Red),
            ("r3", "Eagle", Red),
            ("r2", "Finch", Red),
            ("r1", "Goose", Red),
            ("b3", "Heron", Blue),
            ("b2", "Ibis", Blue),
            ("b1", "Jay", Blue),
            ("g1", "Kite", Green),
            ("n1", "Lark", Navy),
        ];
        let labels = names(&roster.map(|(id, name, _)| (id, name)));
        let votes: Vec<_> = roster
            .into_iter()
            .map(|(id, _, colour)| support::vote(id, colour, 0))
            .collect();
        let shown = page_named(&votes, &[], &[], None, Paint::NoRecord, "r7", &labels);
        let html = render_flock(&shown);
        assert!(html.contains(&format!("12 peers,{}", super::HERO_TAIL)));

        let red_at = html.find("flex:7 1 150px;").expect("red column");
        let blue_at = html.find("flex:3 1 150px;").expect("blue column");
        let green_at = html[blue_at..]
            .find("flex:1 1 150px;")
            .expect("green column")
            + blue_at;
        let navy_at = html[green_at + 1..]
            .find("flex:1 1 150px;")
            .expect("navy column")
            + green_at
            + 1;
        let groups_end = html.find("A group here").expect("group note");
        assert!(red_at < blue_at && blue_at < green_at && green_at < navy_at);
        assert!(navy_at < groups_end);

        let red = &html[red_at..blue_at];
        let blue = &html[blue_at..green_at];
        let green = &html[green_at..navy_at];
        let navy = &html[navy_at..groups_end];
        for (tile, name, count) in [
            (red, "red", 7),
            (blue, "blue", 3),
            (green, "green", 1),
            (navy, "navy", 1),
        ] {
            assert!(tile.contains("min-width:0;border-radius:18px;overflow:hidden"));
            assert!(tile.contains("padding:16px 18px;display:flex;justify-content:space-between"));
            assert!(tile.contains(&format!(
                "<span style=\"font-weight:800;font-size:24px;letter-spacing:-0.02em\">{name}</span><span style=\"font-weight:800;font-size:34px;line-height:1\">{count}</span>"
            )));
            assert!(tile.contains("<ul style=\"list-style:none;margin:0;padding:12px 18px 16px;display:flex;flex-wrap:wrap;gap:6px 14px;font-size:15px;font-weight:600\">"));
        }
        assert_eq!(html.matches("flex:1 1 150px;").count(), 2);
        assert!(html.contains(&format!(
            "<p class=\"muted\" style=\"font-size:14px\">{}</p>",
            super::GROUP_NOTE
        )));
        assert!(red.contains("background:#FF0000;color:#000000"));
        assert!(blue.contains("background:#0000FF;color:#FFFFFF"));
        assert!(green.contains("background:#008000;color:#FFFFFF"));
        assert!(navy.contains("background:#000080;color:#FFFFFF"));
        assert_in_order(
            red,
            &[
                "<li>Avocet</li>",
                "<li>Bunting</li>",
                "<li>Crane</li>",
                "<li>Dunlin</li>",
                "<li>Eagle</li>",
                "<li>Finch</li>",
                "<li>Goose</li>",
            ],
        );
        assert_in_order(blue, &["<li>Heron</li>", "<li>Ibis</li>", "<li>Jay</li>"]);
        assert!(green.contains("<li>Kite</li>"));
        assert!(navy.contains("<li>Lark</li>"));
        assert_eq!(html.matches(&chip(Red)).count(), 7);
        assert_eq!(html.matches(&chip(Blue)).count(), 3);
        assert_eq!(html.matches(&chip(Green)).count(), 1);
        assert_eq!(html.matches(&chip(Navy)).count(), 1);
        assert!(html.contains(&named_peer("Avocet", "r7")));
        assert!(
            html.find(&named_peer("Avocet", "r7")).unwrap()
                < html.find(&named_peer("Goose", "r1")).unwrap()
        );
        assert_rules(&html);
        assert_clean(&html);
    }

    #[test]
    fn trail_changes_and_last_active_follow_the_records() {
        let labels = names(&[
            ("peer-b", "Avocet"),
            ("peer-c", "Bittern"),
            ("peer-a", "Wren"),
        ]);
        let votes = [
            support::vote("peer-b", Blue, 10 * MINUTE_MS),
            support::vote("peer-b", Red, 20 * MINUTE_MS),
            support::vote("peer-b", Blue, 30 * MINUTE_MS),
        ];
        let arguments = [
            argument("peer-b", 40 * MINUTE_MS),
            argument("peer-b", 45 * MINUTE_MS),
            argument("peer-c", 25 * MINUTE_MS),
        ];
        let evidence = [
            evidence("peer-b", 12 * MINUTE_MS),
            evidence("peer-a", 5 * MINUTE_MS),
            evidence("peer-a", 50 * MINUTE_MS),
        ];
        let as_of_ms = 50 * MINUTE_MS;
        let page = DayPage {
            day: Day::from_ms(as_of_ms),
            previous: None,
            next: None,
            votes: &votes,
            arguments: &arguments,
            evidence: &evidence,
            decision: None,
            chrome: Chrome {
                painted_by: Paint::NoRecord,
                as_of_ms,
                rendered_at_ms: 3_723_000,
                server: "peer-a",
                names: &labels,
            },
        };
        let html = render_flock(&page);
        assert!(html.contains("<table style=\"min-width:760px\">"));
        for label in ["CHANGES", "FETCHES", "ARGUMENTS", "LAST ACTIVE"] {
            assert!(html.contains(&format!("<th scope=\"col\" class=\"num\">{label}</th>")));
        }
        assert_eq!(html.matches("<th scope=\"col\" class=\"num\">").count(), 4);
        assert_eq!(html.matches("<td class=\"num\">").count(), 12);
        assert!(html.contains("Grouped by current vote · 00:50 UTC"));
        let groups = html.split("Every peer today").next().unwrap();
        assert!(groups.contains("flex:1 1 150px;"));
        assert!(groups.contains("background:#0000FF;color:#FFFFFF"));
        assert!(groups.contains("<li>Avocet</li>"));
        assert!(!groups.contains("Bittern"));
        assert!(!groups.contains("Wren"));

        let avocet = row_after(&html, &named_peer("Avocet", "peer-b"));
        let chips = format!(
            "<span class=\"chips\" style=\"gap:4px\">{}{}{}</span>",
            chip(Blue),
            chip(Red),
            chip(Blue)
        );
        let chips_at = avocet.find(&chips).expect("blue red blue chips");
        let changes_at = avocet
            .find("<td class=\"num\">2</td>")
            .expect("two changes");
        let fetches_at = avocet.find("<td class=\"num\">1</td>").expect("one fetch");
        let arguments_at = avocet[changes_at + 1..]
            .find("<td class=\"num\">2</td>")
            .expect("two arguments")
            + changes_at
            + 1;
        let last_at = avocet
            .find("<td class=\"num\">00:45</td>")
            .expect("last active");
        assert!(chips_at < changes_at && changes_at < fetches_at && fetches_at < arguments_at);
        assert!(arguments_at < last_at);

        let bittern = row_after(&html, &named_peer("Bittern", "peer-c"));
        assert!(bittern.contains("no vote yet"));
        assert!(bittern.contains("<td class=\"num\">0</td>"));
        assert!(bittern.contains("<td class=\"num\">1</td>"));
        assert!(bittern.contains("<td class=\"num\">00:25</td>"));

        let wren = row_after(&html, &named_peer("Wren", "peer-a"));
        assert!(wren.contains("no vote yet"));
        assert!(wren.contains("<td class=\"num\">2</td>"));
        assert!(wren.contains("<td class=\"num\">00:50</td>"));

        let avocet_at = html.find(&named_peer("Avocet", "peer-b")).unwrap();
        let bittern_at = html.find(&named_peer("Bittern", "peer-c")).unwrap();
        let wren_at = html.find(&named_peer("Wren", "peer-a")).unwrap();
        assert!(avocet_at < bittern_at && bittern_at < wren_at);

        assert!(html.contains(&format!(
            "Records as of {} UTC on {}; rendered {} {} UTC",
            utc_time(as_of_ms),
            Day::from_ms(as_of_ms),
            Day::from_ms(3_723_000),
            utc_time(3_723_000)
        )));
        assert_rules(&html);
        assert_marked_clean(&html);
        assert!(!html.contains("<li><a"));
        assert_eq!(html.matches("<td><a href=\"flock.html\"").count(), 3);
    }

    #[test]
    fn unknown_peer_shows_only_its_escaped_id() {
        let votes = [support::vote("<peer&>", Blue, 0)];
        let html = render_flock(&page(&votes, &[], &[], None, Paint::NoRecord));
        let table = html.split_once("<table ").unwrap().1;
        assert!(table.contains(
            "<td><a href=\"flock.html\" style=\"text-decoration:none;display:flex;flex-direction:column;line-height:1.25\"><span class=\"mono muted\" style=\"font-size:12px\">&lt;peer&amp;&gt;</span></a></td>"
        ));
        assert!(!table.contains("font-weight:700"));
        assert!(!html.contains("<peer&>"));
        assert_rules(&html);
        assert_clean(&html);
    }

    fn named_peer(name: &str, id: &str) -> String {
        format!(
            "<span style=\"font-weight:700\">{name}</span><span class=\"mono muted\" style=\"font-size:12px\">{id}</span>"
        )
    }

    fn row_after<'a>(html: &'a str, marker: &str) -> &'a str {
        html.split_once(marker)
            .unwrap_or_else(|| panic!("missing {marker}"))
            .1
            .split("</tr>")
            .next()
            .unwrap()
    }

    fn assert_in_order(html: &str, needles: &[&str]) {
        let mut from = 0;
        for needle in needles {
            let at = html[from..]
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle} in {html}"))
                + from;
            from = at + needle.len();
        }
    }

    fn argument(author: &str, at_ms: u64) -> ArgumentView {
        ArgumentView {
            id: format!("arg-{author}-{at_ms}"),
            author: author.to_owned(),
            at_ms,
            text: "a claim".to_owned(),
            cites: Vec::new(),
        }
    }

    fn evidence(author: &str, at_ms: u64) -> Evidence {
        Evidence::new(
            author.to_owned(),
            "run".to_owned(),
            Url::parse("https://example.com/fact").unwrap(),
            at_ms,
            "a fact",
        )
    }
}
