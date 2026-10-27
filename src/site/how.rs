//! How it works: one day, the three actions, the colours and the published rules.
//!
//! Every count and rule is a constant the code runs.

use crate::actions::{ARGUMENT_CHARS, FETCHES_PER_ACTIVATION, MAX_CITATIONS};
use crate::closing::{HOLD_MS, HOLDING_RULE, LIVE_RULE, RULE};
use crate::colour::Colour;
use crate::day::CLOSE_AFTER_MS;
use crate::evidence::{EXCERPT_CHARS, SUNRISE_HOST, WIKIMEDIA_HOST};
use crate::tally::{CURRENT_VOTES_RULE, VOTE_CHANGES_RULE};

use super::layout::{document, push_escaped};
use super::{Chrome, Paint, Section};

const HERO: &str = "A question with a worthless answer, so that what you watch is how a swarm \
     gets there. The rules below are the ones the code runs, word for word.";

const WONT_HEADING: &str = "What this site won\u{2019}t tell you";
const WONT_RECORDS: &str = "It shows who voted what and when, what they read, and what they \
     wrote, as exact records with the formula beside every count. It never says why. Whether a \
     group of peers is a side, or whether one message changed a mind, is yours to judge.";
const WONT_PERSONALITY: &str = "No peer has a personality. Anything one peer writes reaches \
     another only as a short, typed, quoted field.";
const WHERE_RUNS: &str = "Each peer runs a 4B-class model on its own machine and keeps a copy \
     of every record; any peer can serve this site. The first runs are in a workstation lab; \
     the plan is Acurast phones on the Liskov platform.";
const WHERE_LINKS: &str = "plumage is built on <a href=\"https://github.com/proof-computer/forthebirds\">forthebirds</a>, \
     the shared machine behind pointless swarms. Its sibling, <a href=\"https://planetarium.proof.computer/\">Planetarium</a>, \
     studies the Acurast network.";

const PUBLISHED: &[(&str, &str)] = &[
    ("Closing rule", RULE),
    ("Live voters", LIVE_RULE),
    ("Holding", HOLDING_RULE),
    ("Current vote", CURRENT_VOTES_RULE),
    ("Vote change", VOTE_CHANGES_RULE),
];

/// How it works for `chrome`. Depth 0, section [`Section::How`].
pub fn render_how(chrome: &Chrome<'_>) -> String {
    let mut body = String::new();
    push_hero(&mut body);
    push_one_day(&mut body, chrome.painted_by);
    push_actions(&mut body);
    push_palette(&mut body);
    push_rules(&mut body);
    push_limits(&mut body);
    document("How it works", Some(Section::How), 0, chrome, &body)
}

fn push_hero(out: &mut String) {
    out.push_str("<h1>How it works</h1>\n<p>");
    out.push_str(HERO);
    out.push_str("</p>\n");
}

fn push_one_day(out: &mut String, paint: Paint) {
    let close = close_label();
    let open = open_hours();
    out.push_str(
        "<section class=\"card\" aria-labelledby=\"day-h\">\n<h2 id=\"day-h\">One day</h2>\n",
    );
    out.push_str("<div role=\"img\" aria-label=\"");
    push_escaped(
        out,
        &format!(
            "A day: open from 00:00 to {close} UTC with peers fetching, arguing and voting; \
             closed from {close}; the decided colour paints the site from the next 00:00."
        ),
    );
    out.push_str("\">\n<div style=\"display:flex;height:56px;border-radius:14px;overflow:hidden;box-shadow:inset 0 0 0 1px rgba(0,0,0,0.12)\">");
    out.push_str(&format!(
        "<div style=\"flex:{open};background:var(--chip)\"></div>"
    ));
    out.push_str(
        "<div style=\"flex:1;background:repeating-linear-gradient(45deg,#D9D8D2 0 4px,#ECEBE6 4px 8px)\"></div>",
    );
    out.push_str("<div class=\"tomorrow\" style=\"");
    out.push_str(&tomorrow_style(paint));
    out.push_str("\">tomorrow</div></div>\n");
    out.push_str("<div class=\"mono\" style=\"display:flex;font-size:12px\">");
    out.push_str(&format!("<span style=\"flex:{open}\">00:00 open</span>"));
    out.push_str(&format!("<span style=\"flex:1\">{close}</span>"));
    out.push_str("<span style=\"flex:4;text-align:right\">00:00 painted</span></div>\n</div>\n");
    out.push_str("<ol style=\"margin:0;padding:0;list-style:none;display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px\">\n");
    push_step(
        out,
        "00:00 UTC",
        "The question opens",
        "The date is the only input. Nothing carries over from yesterday except the paint.",
    );
    push_step(
        out,
        "all day",
        "Peers wake, read, argue, vote",
        "Each wake-up is one activation: a few fetches, maybe an argument, and a vote, which ends it.",
    );
    push_step(
        out,
        &format!("any time before {close}"),
        "A colour may be decided",
        "The first colour held by more than half of the live voters for a continuous hour.",
    );
    push_step(
        out,
        "next 00:00 UTC",
        "The colour paints the site",
        "For one day every page is that colour. An undecided day paints nothing.",
    );
    out.push_str("</ol>\n</section>\n");
}

fn push_step(out: &mut String, when: &str, title: &str, body: &str) {
    out.push_str("<li style=\"display:flex;flex-direction:column;gap:6px\"><span class=\"mono\">");
    out.push_str(when);
    out.push_str("</span><strong>");
    out.push_str(title);
    out.push_str("</strong><span>");
    out.push_str(body);
    out.push_str("</span></li>\n");
}

fn push_actions(out: &mut String) {
    out.push_str("<h2 id=\"act-h\">Three things a peer can do</h2>\n");
    out.push_str("<div style=\"display:flex;flex-wrap:wrap;gap:12px\">\n");
    push_action(
        out,
        "fetch",
        "Read a fact about today from one of two sources.",
        &[
            format!(
                "Wikipedia\u{2019}s \u{201c}On this day\u{201d} feed (<code>{WIKIMEDIA_HOST}</code>), \
                 or London\u{2019}s sunrise and sunset (<code>{SUNRISE_HOST}</code>)"
            ),
            format!("{FETCHES_PER_ACTIVATION} fetches per activation"),
            "Each peer starts at a different event, set by a hash of its id".to_owned(),
            format!(
                "Stored as evidence: URL, time, SHA-256 digest, at most {EXCERPT_CHARS} characters"
            ),
        ],
    );
    push_action(
        out,
        "argue",
        "Say something to the other peers.",
        &[
            format!("At most {ARGUMENT_CHARS} characters"),
            format!("May cite up to {MAX_CITATIONS} pieces of evidence"),
            "Only evidence the peer fetched itself, in this run".to_owned(),
        ],
    );
    push_action(
        out,
        "vote",
        "Name a colour. A peer may change its vote at any time.",
        &[
            format!("Exactly one of the {} names", Colour::ALL.len()),
            "A vote ends the activation".to_owned(),
            "Only the newest vote counts".to_owned(),
        ],
    );
    out.push_str("</div>\n");
}

fn push_action(out: &mut String, verb: &str, what: &str, limits: &[String]) {
    out.push_str(
        "<article class=\"card\" style=\"flex:1 1 300px\">\n<span class=\"mono\" \
         style=\"padding:4px 10px;border-radius:8px;background:#141414;color:#FFFFFF\">",
    );
    out.push_str(verb);
    out.push_str("</span>\n<p style=\"font-weight:600\">");
    out.push_str(what);
    out.push_str("</p>\n<ul>\n");
    for limit in limits {
        out.push_str("<li>");
        out.push_str(limit);
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n</article>\n");
}

fn push_palette(out: &mut String) {
    out.push_str("<section class=\"card\" aria-labelledby=\"pal-h\">\n<h2 id=\"pal-h\">");
    out.push_str(&format!("The {} colours", Colour::ALL.len()));
    out.push_str("</h2>\n<p>A vote must name one of these. Each has one fixed value.</p>\n");
    out.push_str("<ul style=\"list-style:none;margin:0;padding:0;display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px\">\n");
    for colour in Colour::ALL {
        let hex = hex(colour);
        let ink = contrasting_text(colour.srgb());
        out.push_str("<li style=\"border-radius:16px;overflow:hidden;box-shadow:inset 0 0 0 1px rgba(0,0,0,0.12)\">");
        out.push_str(&format!(
            "<span style=\"display:block;height:78px;background:{hex};color:{ink};padding:10px 12px;box-sizing:border-box;font-weight:800\">{name}</span>",
            name = colour.name()
        ));
        out.push_str(&format!(
            "<span class=\"mono\" style=\"display:block;padding:8px 12px\">{hex}</span></li>\n"
        ));
    }
    out.push_str("</ul>\n</section>\n");
}

fn push_rules(out: &mut String) {
    out.push_str("<section aria-labelledby=\"rules-h\" style=\"background:#141414;color:#F6F5F1;border-radius:24px;padding:28px;margin:0 0 20px\">\n");
    out.push_str("<h2 id=\"rules-h\">The rules, as published</h2>\n");
    out.push_str("<dl style=\"margin:0;display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:20px 32px\">\n");
    for (name, text) in PUBLISHED {
        out.push_str("<div><dt>");
        out.push_str(name);
        out.push_str("</dt><dd class=\"mono\" style=\"margin:0;color:#DADAD4\">");
        out.push_str(text);
        out.push_str("</dd></div>\n");
    }
    out.push_str("</dl>\n</section>\n");
}

fn push_limits(out: &mut String) {
    out.push_str("<div style=\"display:flex;flex-wrap:wrap;gap:12px\">\n");
    out.push_str("<section class=\"card\" style=\"flex:1 1 420px\" aria-labelledby=\"never-h\">\n");
    out.push_str("<h2 id=\"never-h\">");
    out.push_str(WONT_HEADING);
    out.push_str("</h2>\n<p>");
    out.push_str(WONT_RECORDS);
    out.push_str("</p>\n<p>");
    out.push_str(WONT_PERSONALITY);
    out.push_str("</p>\n</section>\n");
    out.push_str("<section class=\"card\" style=\"flex:1 1 420px\" aria-labelledby=\"where-h\">\n");
    out.push_str("<h2 id=\"where-h\">Where it runs</h2>\n<p>");
    out.push_str(WHERE_RUNS);
    out.push_str("</p>\n<p>");
    out.push_str(WHERE_LINKS);
    out.push_str("</p>\n</section>\n</div>\n");
}

fn open_hours() -> u64 {
    CLOSE_AFTER_MS / HOLD_MS
}

fn close_label() -> String {
    let hours = open_hours();
    let minutes = (CLOSE_AFTER_MS % HOLD_MS) / (HOLD_MS / 60);
    format!("{hours:02}:{minutes:02}")
}

fn tomorrow_style(paint: Paint) -> String {
    let fill = match paint {
        Paint::Decided(colour) => {
            format!(
                "background:{};color:{}",
                hex(colour),
                contrasting_text(colour.srgb())
            )
        }
        Paint::Undecided | Paint::NoRecord => {
            "background:var(--ground);color:var(--ink)".to_owned()
        }
    };
    format!("flex:4;display:flex;align-items:center;justify-content:center;font-weight:800;{fill}")
}

fn hex(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!("#{red:02X}{green:02X}{blue:02X}")
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        HERO, WHERE_LINKS, WHERE_RUNS, WONT_HEADING, WONT_PERSONALITY, WONT_RECORDS, render_how,
    };
    use crate::actions::{ARGUMENT_CHARS, FETCHES_PER_ACTIVATION, MAX_CITATIONS};
    use crate::closing::{HOLD_MS, HOLDING_RULE, LIVE_RULE, RULE};
    use crate::colour::Colour;
    use crate::day::CLOSE_AFTER_MS;
    use crate::evidence::{EXCERPT_CHARS, SUNRISE_HOST, WIKIMEDIA_HOST};
    use crate::site::support::assert_clean;
    use crate::site::{Chrome, Paint};
    use crate::tally::{CURRENT_VOTES_RULE, VOTE_CHANGES_RULE};

    fn render(paint: Paint) -> String {
        let names = BTreeMap::new();
        render_how(&Chrome {
            painted_by: paint,
            as_of_ms: 0,
            rendered_at_ms: 3_723_000,
            server: "peer-a",
            names: &names,
        })
    }

    fn tomorrow_style(html: &str) -> &str {
        let marker = "class=\"tomorrow\" style=\"";
        let start = html.find(marker).expect("tomorrow segment") + marker.len();
        let end = html[start..].find('"').expect("style end");
        &html[start..start + end]
    }

    #[test]
    fn limits_and_rules_come_from_the_constants() {
        let html = render(Paint::Undecided);
        let close_hours = CLOSE_AFTER_MS / HOLD_MS;
        let close_minutes = (CLOSE_AFTER_MS % HOLD_MS) / (HOLD_MS / 60);
        let close = format!("{close_hours:02}:{close_minutes:02}");
        assert!(html.contains(&format!("{FETCHES_PER_ACTIVATION} fetches per activation")));
        assert!(html.contains(&format!("At most {ARGUMENT_CHARS} characters")));
        assert!(html.contains(&format!("up to {MAX_CITATIONS} pieces of evidence")));
        assert!(html.contains(&format!("at most {EXCERPT_CHARS} characters")));
        assert!(html.contains(&format!("<code>{WIKIMEDIA_HOST}</code>")));
        assert!(html.contains(&format!("<code>{SUNRISE_HOST}</code>")));
        assert!(html.contains("Wikipedia\u{2019}s \u{201c}On this day\u{201d} feed"));
        assert!(html.contains("London\u{2019}s sunrise and sunset"));
        assert!(html.contains(&format!("Exactly one of the {} names", Colour::ALL.len())));
        assert!(html.contains(&format!("The {} colours", Colour::ALL.len())));
        assert!(html.contains(&format!("flex:{close_hours}")));
        assert!(html.contains(&format!("any time before {close}")));
        assert!(html.contains("00:00 open"));
        assert!(html.contains("00:00 painted"));
        assert!(html.contains("repeating-linear-gradient"));
        for (name, text) in [
            ("Closing rule", RULE),
            ("Live voters", LIVE_RULE),
            ("Holding", HOLDING_RULE),
            ("Current vote", CURRENT_VOTES_RULE),
            ("Vote change", VOTE_CHANGES_RULE),
        ] {
            assert!(html.contains(&format!("<dt>{name}</dt>")), "{name}");
            assert!(html.contains(text), "{name}");
        }
    }

    #[test]
    fn palette_names_every_colour_with_its_hex_and_contrasting_ink() {
        let html = render(Paint::NoRecord);
        for colour in Colour::ALL {
            let [red, green, blue] = colour.srgb();
            let hex = format!("#{red:02X}{green:02X}{blue:02X}");
            let ink = super::contrasting_text(colour.srgb());
            assert!(
                html.contains(&format!("background:{hex};color:{ink}")),
                "{hex}"
            );
            assert!(html.contains(&format!(">{hex}</span>")), "{hex}");
            assert!(
                html.contains(&format!(">{name}</span>", name = colour.name())),
                "{}",
                colour.name()
            );
        }
        assert!(html.contains("background:#FF0000;color:#000000"));
        assert!(html.contains("background:#0000FF;color:#FFFFFF"));
    }

    #[test]
    fn tomorrow_takes_the_paint_only_when_decided() {
        let decided_page = render(Paint::Decided(Colour::Red));
        let decided = tomorrow_style(&decided_page);
        assert!(decided.contains("background:#FF0000"));
        assert!(decided.contains("color:#000000"));
        assert!(!decided.contains("var(--ground)"));

        for paint in [Paint::Undecided, Paint::NoRecord] {
            let page = render(paint);
            let style = tomorrow_style(&page);
            assert!(style.contains("background:var(--ground)"), "{paint:?}");
            assert!(style.contains("color:var(--ink)"), "{paint:?}");
            assert!(!style.contains('#'), "{paint:?}: {style}");
        }
    }

    #[test]
    fn pages_are_clean_and_say_what_the_artboard_says() {
        for paint in [
            Paint::Decided(Colour::Red),
            Paint::Undecided,
            Paint::NoRecord,
        ] {
            let html = render(paint);
            assert!(html.contains("<h1>How it works</h1>"));
            assert!(html.contains(HERO));
            assert!(html.contains("href=\"how.html\" aria-current=\"page\""));
            assert!(html.contains("role=\"img\""));
            assert!(html.contains(WONT_HEADING));
            assert!(html.contains(WONT_RECORDS));
            assert!(html.contains(WONT_PERSONALITY));
            assert!(html.contains("Where it runs"));
            assert!(html.contains(WHERE_RUNS));
            assert!(html.contains(WHERE_LINKS));
            assert!(html.contains("url(\"assets/fonts/"));
            assert!(!html.contains("../"));
            assert_clean(&html);
        }
    }
}
