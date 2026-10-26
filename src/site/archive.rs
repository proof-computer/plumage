//! The index of days.

use std::cmp::Reverse;

use super::layout::{chip, document};
use super::{ArchivePage, Paint, Section};

/// The index of days, newest first. Each entry links `<YYYY-MM-DD>.html`.
pub fn render_archive(page: &ArchivePage<'_>) -> String {
    let mut ordered: Vec<&(crate::day::Day, Paint)> = page.days.iter().collect();
    ordered.sort_by_key(|day| Reverse(day.0));
    let mut list = String::from("<ul>\n");
    for (day, paint) in ordered {
        list.push_str("<li><a href=\"");
        list.push_str(&day.to_string());
        list.push_str(".html\">");
        list.push_str(&day.to_string());
        list.push(' ');
        match paint {
            Paint::Decided(colour) => list.push_str(&chip(*colour)),
            Paint::Undecided | Paint::NoRecord => list.push_str("undecided"),
        }
        list.push_str("</a></li>\n");
    }
    list.push_str("</ul>\n");
    let mut body = String::from("<h1>Archive</h1>\n<div class=\"card\">\n");
    body.push_str(&list);
    body.push_str("</div>\n");
    document("Archive", Some(Section::Days), 1, &page.chrome, &body)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::render_archive;
    use crate::colour::Colour::Blue;
    use crate::site::support::FORBIDDEN;
    use crate::site::{ArchivePage, Chrome, Paint};

    #[test]
    fn render_archive_lists_days_newest_first() {
        let days = [
            ("2025-10-26".parse().unwrap(), Paint::Decided(Blue)),
            ("2025-10-27".parse().unwrap(), Paint::Undecided),
            ("2025-10-25".parse().unwrap(), Paint::NoRecord),
        ];
        let names = BTreeMap::new();
        let html = render_archive(&ArchivePage {
            days: &days,
            chrome: Chrome {
                painted_by: Paint::NoRecord,
                as_of_ms: 0,
                rendered_at_ms: 3_723_000,
                server: "peer-a",
                names: &names,
            },
        });
        let newest = html.find("href=\"2025-10-27.html\"").unwrap();
        let middle = html.find("href=\"2025-10-26.html\"").unwrap();
        let oldest = html.find("href=\"2025-10-25.html\"").unwrap();
        assert!(newest < middle && middle < oldest);
        assert!(!html.contains("days/2025-10-27.html"));
        assert!(!html.contains("days/2025-10-26.html"));
        assert!(!html.contains("days/2025-10-25.html"));
        assert!(html.contains("href=\"../days/index.html\" aria-current=\"page\""));
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
