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
a.wordmark { display: flex; flex-direction: column; gap: 8px; width: fit-content; color: inherit; text-decoration: none; font-size: 30px; font-weight: 800; letter-spacing: -0.03em; line-height: 1; }
nav[aria-label=\"Site\"] { display: flex; flex-wrap: wrap; gap: 8px; }
nav[aria-label=\"Site\"] a { color: inherit; text-decoration: none; padding: 10px 16px; border-radius: 999px; font-size: 16px; font-weight: 600; }
.band { display: flex; height: 10px; border-radius: 5px; overflow: hidden; box-shadow: 0 0 0 1px rgba(0,0,0,0.12); }
.band span { flex: 1 1 0; }
a.wordmark .band { width: 132px; height: 6px; border-radius: 3px; }
.card { background: var(--card); color: var(--ink); border-radius: 24px; padding: 28px; box-sizing: border-box; margin: 0 0 20px; }
.chip { display: inline-block; padding: 1px 9px; border-radius: 999px; background: var(--chip); font-weight: 600; }
.mono, code, .formula p { font-family: \"IBM Plex Mono\", ui-monospace, monospace; }
.muted { color: var(--muted); }
table { border-collapse: collapse; }
td, th { border: 1px solid var(--hairline); padding: 0.2rem; vertical-align: top; }
details summary { list-style: none; }
details summary::-webkit-details-marker { display: none; }
details summary::marker { content: none; }
@media (max-width: 639px) {
  .wrap { padding-left: 16px; padding-right: 16px; }
  h1 { font-size: 42px; }
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
    out.push('\n');
    push_freshness(out, chrome);
    out.push_str(
        "<p><a href=\"https://github.com/proof-computer/plumage\">Source</a> <a href=\"https://github.com/proof-computer/forthebirds\">forthebirds</a> <a href=\"https://planetarium.proof.computer/\">Planetarium</a></p>\n</footer>\n",
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
    out.push_str("<p>");
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
    out.push_str("</p>\n");
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
