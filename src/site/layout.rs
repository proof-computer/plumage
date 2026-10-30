//! The shared painted shell: fonts, masthead, band, cards and footer.
//!
//! `@font-face` uses the CSS descriptor `src:`. The site test forbids the HTML
//! attribute `src=`, which this string does not contain.

use crate::colour::Colour;
use crate::day::{Day, utc_time};

use super::{Chrome, Paint, Section};

const FACES: &[(&str, &str, &str)] = &[
    (
        "Bricolage Grotesque",
        "200 800",
        "bricolage-grotesque-latin-opsz-normal.woff2",
    ),
    (
        "IBM Plex Mono",
        "400",
        "ibm-plex-mono-latin-400-normal.woff2",
    ),
    (
        "IBM Plex Mono",
        "500",
        "ibm-plex-mono-latin-500-normal.woff2",
    ),
];

const NAV: &[(&str, &str, Section)] = &[
    ("index.html", "Today", Section::Today),
    ("days/index.html", "The days", Section::Days),
    ("flock.html", "The flock", Section::Flock),
    ("how.html", "How it works", Section::How),
];

const STYLESHEET: &str = "\
:root { --ink: #141414; --card: #FFFFFF; --muted: #5C5C5C; --hairline: #ECECE8; --chip: #F3F3F0; --ground: #F3F2EE; }
body { margin: 0; background: var(--ground); color: var(--ink); font-family: \"Bricolage Grotesque\", system-ui, sans-serif; font-size: 17px; line-height: 1.5; }
.wrap { max-width: 1200px; margin: 0 auto; padding: 0 40px; box-sizing: border-box; }
header.wrap { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 20px; padding-top: 28px; }
main.wrap { padding-bottom: 48px; }
footer.wrap { display: flex; flex-direction: column; gap: 20px; padding-bottom: 40px; }
h1 { margin: 0.4em 0; font-size: 76px; line-height: 0.98; font-weight: 800; letter-spacing: -0.035em; }
h2 { margin: 0 0 16px; font-size: 26px; letter-spacing: -0.02em; }
a { color: inherit; text-underline-offset: 3px; }
a.wordmark { display: flex; flex-direction: column; gap: 8px; width: fit-content; color: inherit; text-decoration: none; font-size: 30px; font-weight: 800; letter-spacing: -0.03em; line-height: 1; }
nav[aria-label=\"Site\"] { display: flex; flex-wrap: wrap; gap: 8px; }
nav[aria-label=\"Site\"] a { color: inherit; text-decoration: none; padding: 10px 16px; border-radius: 999px; font-size: 16px; font-weight: 600; }
.band { display: flex; height: 10px; border-radius: 5px; overflow: hidden; box-shadow: 0 0 0 1px rgba(0,0,0,0.12); }
.band span { flex: 1 1 0; }
a.wordmark .band { width: 132px; height: 6px; border-radius: 3px; }
.card { background: var(--card); color: var(--ink); border-radius: 24px; padding: 28px; box-sizing: border-box; margin: 0 0 20px; box-shadow: 0 1px 0 rgba(0,0,0,0.08); }
.chip { display: inline-block; padding: 1px 9px; border-radius: 999px; background: var(--chip); font-weight: 600; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.15); font-size: 13px; }
.mono, code, .formula p { font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
code { overflow-wrap: anywhere; }
.muted { color: var(--muted); }
.hero { display: flex; flex-wrap: wrap; gap: 40px; align-items: flex-end; padding: 72px 0 48px; }
.hero-text { flex: 999 1 520px; min-width: 0; display: flex; flex-direction: column; gap: 20px; }
.hero-card { flex: 1 1 380px; display: flex; flex-direction: column; gap: 18px; margin: 0; box-shadow: 0 1px 0 rgba(0,0,0,0.08), 0 18px 40px -24px rgba(0,0,0,0.45); }
.lede { font-size: 21px; max-width: 34em; line-height: 1.45; }
.eyebrow { font-size: 14px; letter-spacing: 0.02em; }
.split { display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start; }
.split > .main { flex: 999 1 560px; min-width: 0; }
.split > .side { flex: 1 1 340px; min-width: 0; display: flex; flex-direction: column; gap: 12px; }
nav[aria-label=\"Days\"] { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 12px; padding-top: 40px; font-size: 15px; font-weight: 600; }
.big { font-size: 52px; font-weight: 800; letter-spacing: -0.03em; line-height: 1; }
.swatch { display: inline-block; width: 64px; height: 64px; border-radius: 50%; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.12); flex: none; }
.swatch-sm { display: inline-block; width: 20px; height: 20px; border-radius: 50%; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.15); flex: none; }
.swatch-xs { display: inline-block; width: 14px; height: 14px; border-radius: 4px; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.15); flex: none; }
.bar { position: relative; height: 16px; border-radius: 8px; background: #EDEDEA; overflow: hidden; }
.bar .fill { position: absolute; left: 0; top: 0; bottom: 0; }
.bar .tick { position: absolute; top: 0; bottom: 0; width: 2px; background: #FFFFFF; }
.bar-labels { display: flex; justify-content: space-between; font-size: 13px; color: var(--muted); }
.legend { display: flex; flex-wrap: wrap; gap: 14px; font-size: 14px; }
.legend span { display: inline-flex; align-items: center; gap: 6px; }
.chips { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.chip-lg { display: inline-flex; align-items: center; gap: 8px; padding: 6px 12px 6px 6px; border-radius: 999px; background: var(--chip); font-weight: 600; font-size: 15px; }
.tiles { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
.tile { flex: 1 1 78px; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
.tile .paint { height: 96px; border-radius: 14px; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.12); display: flex; align-items: flex-end; padding: 10px; box-sizing: border-box; font-size: 12px; font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.tile a { display: flex; flex-direction: column; text-decoration: none; line-height: 1.25; }
.tile .name { font-weight: 700; font-size: 15px; }
.tile .id { font-size: 11px; color: var(--muted); overflow-wrap: anywhere; font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.numbers { display: flex; flex-wrap: wrap; gap: 12px; margin: 16px 0; }
.numbers .card { flex: 1 1 190px; margin: 0; border-radius: 20px; padding: 18px 20px; display: flex; flex-direction: column; gap: 4px; }
.numbers .n { font-size: 40px; font-weight: 800; letter-spacing: -0.03em; line-height: 1.05; }
.numbers .label { font-size: 15px; font-weight: 600; }
.timeline { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
.timeline li { display: grid; grid-template-columns: 56px 28px minmax(0, 1fr); column-gap: 10px; padding-bottom: 22px; }
.timeline .t { font-size: 13px; color: var(--muted); padding-top: 3px; font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.timeline .rail { display: flex; flex-direction: column; align-items: center; gap: 4px; }
.timeline .dot { width: 16px; height: 16px; border-radius: 50%; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.18); margin-top: 3px; flex: none; }
.timeline .line { width: 2px; flex: 1; background: var(--hairline); }
.timeline .body { min-width: 0; display: flex; flex-direction: column; gap: 8px; font-size: 15px; overflow-wrap: anywhere; }
.timeline blockquote { margin: 0; font-size: 19px; line-height: 1.42; }
.timeline .cite { font-size: 12px; padding: 4px 9px; border-radius: 8px; background: var(--chip); text-decoration: none; font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.timeline .mark { font-size: 14px; font-weight: 600; padding: 8px 12px; border-radius: 10px; background: var(--chip); align-self: flex-start; }
.fact { margin: 0; border-radius: 20px; padding: 18px 20px; display: flex; flex-direction: column; gap: 10px; }
.fact .meta { display: flex; justify-content: space-between; gap: 8px; font-size: 12px; color: var(--muted); font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.fact .excerpt { margin: 0; white-space: pre-line; font-size: 17px; line-height: 1.4; }
.fact .foot { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 6px; font-size: 13px; color: var(--muted); }
table { border-collapse: collapse; width: 100%; font-size: 15px; }
th { font-size: 12px; font-weight: 500; text-align: left; padding: 8px 12px; border-bottom: 1.5px solid var(--ink); font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
td { padding: 12px; border-bottom: 1px solid var(--hairline); vertical-align: top; }
th:first-child, td:first-child { padding-left: 0; }
th:last-child, td:last-child { padding-right: 0; }
.num { text-align: right; font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
details summary { list-style: none; cursor: pointer; font-weight: 600; color: #3A3A3A; }
details p { margin: 10px 0 0; font-size: 13px; line-height: 1.55; color: #3A3A3A; }
.formula summary { font-size: 13px; font-weight: 400; color: var(--muted); }
.formula p { margin: 6px 0 0; font-size: 12px; line-height: 1.5; }
.side > .formula summary, .side > .formula p { color: inherit; }
details summary::-webkit-details-marker { display: none; }
details summary::marker { content: none; }
footer .foot { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 16px; font-size: 15px; }
footer .foot p { margin: 0; max-width: 36em; }
footer nav { display: flex; flex-wrap: wrap; gap: 18px; font-weight: 600; }
.verdict { font-size: clamp(56px, 18.5vw, 132px); line-height: 0.9; letter-spacing: -0.045em; }
@media (max-width: 639px) {
  .wrap { padding-left: 16px; padding-right: 16px; }
  h1 { font-size: 42px; }
  .hero { padding: 40px 0 32px; }
}
";

/// A full HTML document. `depth` 0 is the site root; each level prefixes `../`.
pub(crate) fn document(
    title: &str,
    section: Option<Section>,
    depth: u8,
    chrome: &Chrome<'_>,
    body: &str,
) -> String {
    let prefix = "../".repeat(usize::from(depth));
    let mut html = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
    );
    push_escaped(&mut html, title);
    html.push_str("</title>\n<style>\n");
    for (family, weight, file) in FACES {
        html.push_str(&format!(
            "@font-face {{ font-family: \"{family}\"; font-weight: {weight}; src: url(\"{prefix}assets/fonts/{file}\") format(\"woff2\"); font-display: swap; }}\n"
        ));
    }
    html.push_str(STYLESHEET);
    html.push_str("</style>\n</head>\n");
    match paint_style(chrome.painted_by) {
        Some(style) => {
            html.push_str("<body style=\"");
            html.push_str(&style);
            html.push_str("\">\n");
        }
        None => html.push_str("<body>\n"),
    }
    push_header(&mut html, &prefix, section, chrome.painted_by);
    html.push_str("<main class=\"wrap\">\n");
    html.push_str(body);
    html.push_str("</main>\n");
    push_footer(&mut html, chrome);
    html.push_str("</body>\n</html>\n");
    html
}

pub(crate) fn card(heading: &str, body: &str) -> String {
    let mut html = String::from("<section class=\"card\">\n<h2>");
    push_escaped(&mut html, heading);
    html.push_str("</h2>\n");
    html.push_str(body);
    html.push_str("</section>\n");
    html
}

/// A disclosure whose summary is the word formula and whose body is `rule`.
pub fn formula(rule: &str) -> String {
    let mut html =
        String::from("<details class=\"formula\"><summary>formula</summary><p class=\"mono\">");
    push_escaped(&mut html, rule);
    html.push_str("</p></details>");
    html
}

/// The peer's display name, then its id. An unknown id is the id alone.
pub(crate) fn peer(chrome: &Chrome<'_>, id: &str) -> String {
    let mut html = String::new();
    if let Some(name) = chrome.names.get(id) {
        push_escaped(&mut html, name);
        html.push(' ');
    }
    html.push_str("<code>");
    push_escaped(&mut html, id);
    html.push_str("</code>");
    html
}

/// A pill filled with `colour` and labelled with its name.
pub(crate) fn chip(colour: Colour) -> String {
    let [red, green, blue] = colour.srgb();
    format!(
        "<span class=\"chip\" style=\"background:#{red:02X}{green:02X}{blue:02X};color:{}\">{}</span>",
        contrasting_text(colour.srgb()),
        colour.name()
    )
}

pub(crate) fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

fn push_header(out: &mut String, prefix: &str, section: Option<Section>, paint: Paint) {
    let (ink, fill) = ink_and_fill(paint);
    out.push_str("<header class=\"wrap\">\n<a class=\"wordmark\" href=\"");
    out.push_str(prefix);
    out.push_str("index.html\">plumage");
    push_band(out);
    out.push_str("</a>\n<nav aria-label=\"Site\">");
    for (href, label, item) in NAV {
        out.push_str("<a href=\"");
        out.push_str(prefix);
        out.push_str(href);
        out.push('"');
        if section == Some(*item) {
            out.push_str(" aria-current=\"page\" style=\"background:");
            out.push_str(&ink);
            out.push_str(";color:");
            out.push_str(&fill);
            out.push_str("\">");
        } else {
            out.push_str(" style=\"box-shadow:inset 0 0 0 1.5px ");
            out.push_str(&ink);
            out.push_str("\">");
        }
        out.push_str(label);
        out.push_str("</a>");
    }
    out.push_str("</nav>\n</header>\n");
}

fn push_footer(out: &mut String, chrome: &Chrome<'_>) {
    out.push_str("<footer class=\"wrap\">\n");
    push_band(out);
    out.push_str("\n<div class=\"foot\"><p>plumage is a pointless swarm. The colour is not the point; watching a swarm agree is. ");
    push_freshness(out, chrome);
    out.push_str(
        "</p><nav aria-label=\"More\"><a href=\"https://github.com/proof-computer/plumage\">Source</a> <a href=\"https://github.com/proof-computer/forthebirds\">forthebirds</a> <a href=\"https://planetarium.proof.computer/\">Planetarium</a></nav></div>\n</footer>\n",
    );
}

fn push_band(out: &mut String) {
    out.push_str("<span class=\"band\">");
    for colour in Colour::ALL {
        let [red, green, blue] = colour.srgb();
        out.push_str(&format!(
            "<span style=\"background:#{red:02X}{green:02X}{blue:02X}\" aria-hidden=\"true\"></span>"
        ));
    }
    out.push_str("</span>");
}

fn push_freshness(out: &mut String, chrome: &Chrome<'_>) {
    out.push_str(&format!(
        "Records as of {} UTC on {}; rendered {} {} UTC",
        utc_time(chrome.as_of_ms),
        Day::from_ms(chrome.as_of_ms),
        Day::from_ms(chrome.rendered_at_ms),
        utc_time(chrome.rendered_at_ms)
    ));
    out.push_str(" by ");
    if let Some(name) = chrome.names.get(chrome.server) {
        push_escaped(out, name);
        out.push_str(" (");
        push_escaped(out, chrome.server);
        out.push(')');
    } else {
        push_escaped(out, chrome.server);
    }
}

fn ink_and_fill(paint: Paint) -> (String, String) {
    match paint {
        Paint::Decided(colour) => {
            let [red, green, blue] = colour.srgb();
            (
                contrasting_text(colour.srgb()).to_owned(),
                format!("#{red:02X}{green:02X}{blue:02X}"),
            )
        }
        Paint::Undecided | Paint::NoRecord => ("#141414".to_owned(), "#F3F2EE".to_owned()),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FACES, chip, document, formula};
    use crate::colour::Colour;
    use crate::site::support::{assert_clean, body_open};
    use crate::site::{Chrome, Paint, Section};

    fn sample(paint: Paint, names: &BTreeMap<String, String>) -> Chrome<'_> {
        Chrome {
            painted_by: paint,
            as_of_ms: 0,
            rendered_at_ms: 3_723_000,
            server: "peer-a",
            names,
        }
    }

    #[test]
    fn document_styles_every_shared_component_once() {
        let names = BTreeMap::new();
        let html = document("Today", None, 0, &sample(Paint::NoRecord, &names), "");
        let style = html.split_once("<style>").unwrap().1;
        let style = style.split_once("</style>").unwrap().0;
        let (base, phone) = style.split_once("@media (max-width: 639px)").unwrap();
        for selector in [
            ".hero",
            ".hero-text",
            ".hero-card",
            ".lede",
            ".eyebrow",
            ".split",
            ".split > .main",
            ".split > .side",
            "nav[aria-label=\"Days\"]",
            "h2",
            ".card",
            ".big",
            "code",
            "a",
            ".swatch",
            ".swatch-sm",
            ".swatch-xs",
            ".bar",
            ".bar .fill",
            ".bar .tick",
            ".bar-labels",
            ".legend",
            ".legend span",
            ".chips",
            ".chip",
            ".chip-lg",
            ".tiles",
            ".tile",
            ".tile .paint",
            ".tile a",
            ".tile .name",
            ".tile .id",
            ".numbers",
            ".numbers .card",
            ".numbers .n",
            ".numbers .label",
            ".timeline",
            ".timeline li",
            ".timeline .t",
            ".timeline .rail",
            ".timeline .dot",
            ".timeline .line",
            ".timeline .body",
            ".timeline blockquote",
            ".timeline .cite",
            ".timeline .mark",
            ".fact",
            ".fact .meta",
            ".fact .excerpt",
            ".fact .foot",
            "table",
            "th",
            "td",
            "th:first-child, td:first-child",
            "th:last-child, td:last-child",
            ".num",
            "details summary",
            "details p",
            ".formula summary",
            ".formula p",
            ".side > .formula summary, .side > .formula p",
            ".verdict",
            "footer .foot",
            "footer .foot p",
            "footer nav",
        ] {
            let count = base
                .split('}')
                .filter(|rule| {
                    rule.split_once('{')
                        .is_some_and(|(head, _)| head.trim() == selector)
                })
                .count();
            assert_eq!(count, 1, "selector {selector}");
        }
        for (selector, declaration) in [
            (".timeline .body", "overflow-wrap: anywhere"),
            (
                ".side > .formula summary, .side > .formula p",
                "color: inherit",
            ),
            (".verdict", "clamp(56px, 18.5vw, 132px)"),
        ] {
            let rule = base
                .split('}')
                .filter_map(|rule| rule.split_once('{'))
                .find_map(|(head, body)| (head.trim() == selector).then_some(body))
                .unwrap();
            assert!(rule.contains(declaration), "{selector}: {rule}");
        }
        assert!(!phone.contains(".verdict"));
        assert!(phone.contains(".hero { padding: 40px 0 32px; }"));
        assert!(!style.contains("td, th { border: 1px solid"));
        assert_clean(&html);
    }

    #[test]
    fn depth_prefixes_fonts_and_masthead_and_marks_one_section() {
        let names = BTreeMap::new();
        let chrome = sample(Paint::NoRecord, &names);
        let root = document("Today", Some(Section::Today), 0, &chrome, "");
        let nested = document("Day", Some(Section::Days), 1, &chrome, "");
        for (_, _, file) in FACES {
            assert!(root.contains(&format!("url(\"assets/fonts/{file}\")")));
            assert!(nested.contains(&format!("url(\"../assets/fonts/{file}\")")));
            assert!(!root.contains(&format!("url(\"../assets/fonts/{file}\")")));
        }
        assert!(root.contains("href=\"index.html\" aria-current=\"page\""));
        assert!(root.contains("href=\"days/index.html\""));
        assert!(root.contains("href=\"flock.html\""));
        assert!(root.contains("href=\"how.html\""));
        assert!(!root.contains("../"));
        assert!(nested.contains("href=\"../days/index.html\" aria-current=\"page\""));
        assert!(nested.contains("href=\"../index.html\""));
        assert!(nested.contains("href=\"../flock.html\""));
        assert!(nested.contains("href=\"../how.html\""));
        assert_eq!(root.matches("aria-current=\"page\"").count(), 1);
        assert_eq!(nested.matches("aria-current=\"page\"").count(), 1);
        assert!(!root.contains("src="));
        assert!(!nested.contains("src="));
        assert_clean(&root);
    }

    #[test]
    fn body_style_follows_the_paint_and_the_band_has_every_colour() {
        let names = BTreeMap::new();
        let blue = document(
            "Blue",
            Some(Section::Today),
            0,
            &sample(Paint::Decided(Colour::Blue), &names),
            "",
        );
        let yellow = document(
            "Yellow",
            Some(Section::Today),
            0,
            &sample(Paint::Decided(Colour::Yellow), &names),
            "",
        );
        let undecided = document(
            "Undecided",
            Some(Section::Today),
            0,
            &sample(Paint::Undecided, &names),
            "",
        );
        let missing = document(
            "Missing",
            Some(Section::Today),
            0,
            &sample(Paint::NoRecord, &names),
            "",
        );
        assert!(blue.contains("<body style=\"background:#0000FF;color:#FFFFFF\">"));
        assert!(yellow.contains("<body style=\"background:#FFFF00;color:#000000\">"));
        assert!(!yellow.contains("color:#FFFFFF"));
        assert_eq!(body_open(&undecided), "<body>");
        assert_eq!(body_open(&missing), "<body>");

        let mut band = String::from("<span class=\"band\">");
        for colour in Colour::ALL {
            let [red, green, blue_channel] = colour.srgb();
            band.push_str(&format!(
                "<span style=\"background:#{red:02X}{green:02X}{blue_channel:02X}\" aria-hidden=\"true\"></span>"
            ));
        }
        band.push_str("</span>");
        assert_eq!(blue.matches(&band).count(), 2);
        assert_eq!(undecided.matches("aria-hidden=\"true\"").count(), 48);
    }

    #[test]
    fn footer_names_the_server_and_formula_and_chip_escape() {
        let mut names = BTreeMap::new();
        names.insert("peer-a".to_owned(), "Wren".to_owned());
        let named = document(
            "Today",
            Some(Section::Today),
            0,
            &sample(Paint::NoRecord, &names),
            "",
        );
        assert!(named.contains(
            "Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC by Wren (peer-a)"
        ));
        assert!(named.contains(
            "<div class=\"foot\"><p>plumage is a pointless swarm. The colour is not the point; watching a swarm agree is. Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC by Wren (peer-a)</p><nav aria-label=\"More\">"
        ));
        let more = named.split_once("<nav aria-label=\"More\">").unwrap().1;
        let more = more.split_once("</nav>").unwrap().0;
        assert_eq!(more.matches("<a ").count(), 3);
        for (href, label) in [
            ("https://github.com/proof-computer/plumage", "Source"),
            (
                "https://github.com/proof-computer/forthebirds",
                "forthebirds",
            ),
            ("https://planetarium.proof.computer/", "Planetarium"),
        ] {
            assert!(more.contains(&format!("<a href=\"{href}\">{label}</a>")));
        }

        let unnamed = document(
            "Today",
            Some(Section::Today),
            0,
            &sample(Paint::NoRecord, &BTreeMap::new()),
            "",
        );
        assert!(unnamed.contains(
            "Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC by peer-a"
        ));
        assert!(!unnamed.contains("(peer-a)"));

        let mut escaped = BTreeMap::new();
        escaped.insert("peer-a".to_owned(), "<Wren&>".to_owned());
        let unsafe_name = document(
            "Today",
            Some(Section::Today),
            0,
            &sample(Paint::NoRecord, &escaped),
            "",
        );
        assert!(unsafe_name.contains("by &lt;Wren&amp;&gt; (peer-a)"));
        assert!(!unsafe_name.contains("<Wren&>"));

        let disclosed = formula("a < b & c");
        assert!(disclosed.contains("<summary>formula</summary>"));
        assert!(disclosed.contains("a &lt; b &amp; c"));
        assert!(!disclosed.contains("a < b"));

        let pill = chip(Colour::Blue);
        assert!(pill.contains("background:#0000FF"));
        assert!(pill.contains("color:#FFFFFF"));
        assert!(pill.contains(">blue</span>"));
    }
}
