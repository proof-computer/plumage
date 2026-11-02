//! Self-contained HTML for the plumage site.
//!
//! Pure functions of the records the caller passes. No file, network, store
//! or clock. [`Chrome::rendered_at_ms`] is the caller's clock.

mod archive;
mod day;
mod flock;
mod front;
mod how;
mod layout;
mod summary;

pub use archive::render_archive;
pub use day::render_day;
pub use flock::render_flock;
pub use front::render_front;
pub use how::render_how;
pub use layout::formula;
pub use summary::{
    Activations, MAX_AUTHORS, MAX_STATUS_BYTES, Phase, RECENT_WINDOW, SCHEMA, StatusInput,
    render_status,
};

use std::collections::BTreeMap;

use crate::closing::{Decision, Vote};
use crate::colour::Colour;
use crate::day::Day;
use crate::evidence::Evidence;

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

/// Paint, clock and names shared by every page.
///
/// `names` maps a peer id to its display name. An id missing from it is shown
/// as the id alone.
#[derive(Clone, Copy, Debug)]
pub struct Chrome<'a> {
    pub painted_by: Paint,
    pub as_of_ms: u64,
    pub rendered_at_ms: u64,
    pub server: &'a str,
    pub names: &'a BTreeMap<String, String>,
}

/// The records and clock readings for one day's pages.
#[derive(Debug)]
pub struct DayPage<'a> {
    pub day: Day,
    /// Nearest recorded day before this page; absent before the first record.
    pub previous: Option<(Day, Paint)>,
    /// Nearest later record, or the open day with `Paint::NoRecord`.
    pub next: Option<(Day, Paint)>,
    pub votes: &'a [Vote],
    pub stances: &'a [crate::tally::Stance],
    pub arguments: &'a [ArgumentView],
    pub evidence: &'a [Evidence],
    pub decision: Option<Decision>,
    pub chrome: Chrome<'a>,
}

/// The index of days, newest first when rendered.
#[derive(Debug)]
pub struct ArchivePage<'a> {
    pub days: &'a [(Day, Paint)],
    pub chrome: Chrome<'a>,
}

/// Which masthead link is the current page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Today,
    Days,
    Flock,
    How,
}

/// Font files and their licences, published under the site root.
///
/// The CSS `src:` descriptor points at these paths. It is not an HTML `src`
/// attribute.
pub const ASSETS: [(&str, &[u8]); 5] = [
    (
        "assets/fonts/bricolage-grotesque-latin-opsz-normal.woff2",
        include_bytes!("../../assets/fonts/bricolage-grotesque-latin-opsz-normal.woff2"),
    ),
    (
        "assets/fonts/ibm-plex-mono-latin-400-normal.woff2",
        include_bytes!("../../assets/fonts/ibm-plex-mono-latin-400-normal.woff2"),
    ),
    (
        "assets/fonts/ibm-plex-mono-latin-500-normal.woff2",
        include_bytes!("../../assets/fonts/ibm-plex-mono-latin-500-normal.woff2"),
    ),
    (
        "assets/fonts/LICENSE-BricolageGrotesque.txt",
        include_bytes!("../../assets/fonts/LICENSE-BricolageGrotesque.txt"),
    ),
    (
        "assets/fonts/LICENSE-IBMPlexMono.txt",
        include_bytes!("../../assets/fonts/LICENSE-IBMPlexMono.txt"),
    ),
];

#[cfg(test)]
mod support {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use url::Url;

    use super::{ArgumentView, Chrome, DayPage, Paint};
    use crate::closing::{Decision, Vote};
    use crate::colour::Colour;
    use crate::day::Day;
    use crate::evidence::Evidence;

    pub(crate) const RAW: &str = "</p><script>alert(1)</script> & \"x\"";
    pub(crate) const ESCAPED: &str =
        "&lt;/p&gt;&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;x&quot;";
    pub(crate) const FORBIDDEN: &[&str] = &[
        "faction",
        "persuasion",
        "persuaded",
        "convinced",
        "camp",
        "coalition",
        "winner",
    ];

    fn empty_names() -> &'static BTreeMap<String, String> {
        static NAMES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
        NAMES.get_or_init(BTreeMap::new)
    }

    pub(crate) fn vote(author: &str, colour: Colour, at_ms: u64) -> Vote {
        Vote {
            author: author.to_owned(),
            colour,
            at_ms,
        }
    }

    pub(crate) fn stance(
        author: &str,
        at_ms: u64,
        prefer: &[Colour],
        accept: &[Colour],
        refuse: &[Colour],
    ) -> crate::tally::Stance {
        crate::tally::Stance {
            id: format!("stance-{author}"),
            author: author.to_owned(),
            at_ms,
            prefer: prefer.to_vec(),
            accept: accept.to_vec(),
            refuse: refuse.to_vec(),
            cites: vec![],
        }
    }

    pub(crate) fn stance_fixture() -> (Vec<Vote>, Vec<crate::tally::Stance>) {
        use Colour::{Blue, Green, Navy, Red, Sky, Teal};
        let hour = 3_600_000;
        (
            vec![
                vote("a", Blue, 14 * hour),
                vote("b", Blue, 14 * hour + 600_000),
                vote("c", Green, 14 * hour + 1_200_000),
                vote("d", Red, 11 * hour),
                // Interrupt the early red hold, so Today is still open at 14:30.
                vote("a", Green, 12 * hour - 60_000),
            ],
            vec![
                stance("a", 13 * hour, &[Green], &[Teal, Blue], &[Red]),
                stance("b", 13 * hour, &[Blue, Navy], &[], &[Green]),
                stance("c", 13 * hour, &[Green, Teal], &[Sky], &[]),
                stance("d", 10 * hour, &[Red], &[], &[Blue]),
            ],
        )
    }

    pub(crate) fn stance_panel(html: &str) -> &str {
        let marker = "<section class=\"card stances\"";
        assert_eq!(html.matches(marker).count(), 1);
        let start = html.find(marker).unwrap();
        let end = start + html[start..].find("</section>").unwrap();
        &html[start..end]
    }

    /// Pin every row, including the zeros, against independently named counts.
    pub(crate) fn assert_stance_rows(html: &str, live: usize, expected: &[(Colour, [usize; 3])]) {
        let panel = stance_panel(html);
        let rows: Vec<_> = panel.split("<tr class=\"stance\" ").skip(1).collect();
        assert_eq!(rows.len(), Colour::ALL.len());
        for (row, colour) in rows.iter().zip(Colour::ALL) {
            let row = row.split_once("</tr>").unwrap().0;
            assert!(row.starts_with(&format!("data-colour=\"{}\">", colour.name())));
            assert_eq!(row.matches("<td class=\"band\"").count(), 3);
            let counts = expected
                .iter()
                .find(|(c, _)| *c == colour)
                .map_or([0; 3], |(_, n)| *n);
            let mut remaining = row;
            for (label, count) in ["preferred", "accepted", "refused"].into_iter().zip(counts) {
                remaining = remaining
                    .split_once(&format!(
                        "data-band=\"{label}\" data-count=\"{count}\" data-of=\"{live}\""
                    ))
                    .unwrap()
                    .1;
                let cell = remaining.split_once("</td>").unwrap().0;
                assert!(cell.contains(&format!(">{count}</span>")));
                if live == 0 || count == 0 {
                    assert!(!cell.contains("class=\"stance-fill"));
                }
            }
        }
        let disclosure = panel
            .split_once("<details><summary>How these are counted</summary>")
            .unwrap()
            .1;
        assert!(!disclosure.contains("<tr"));
        for rule in [crate::tally::STANCE_BANDS_RULE, crate::closing::LIVE_RULE] {
            let mut escaped = String::new();
            super::layout::push_escaped(&mut escaped, rule);
            assert!(disclosure.contains(&format!("<p class=\"mono\">{escaped}</p>")));
        }
        for word in FORBIDDEN.iter().chain(
            [
                concat!("com", "promise"),
                concat!("inter", "section"),
                concat!("recom", "mend"),
                "settle",
                "bloc",
            ]
            .iter(),
        ) {
            assert!(!panel.to_lowercase().contains(word));
        }
        let lower = html.to_ascii_lowercase();
        for marker in ["<script", "<link", "src="] {
            assert!(!lower.contains(marker));
        }
        for word in FORBIDDEN {
            assert!(!lower.contains(word));
        }
    }

    pub(crate) fn evidence(excerpt: &str) -> Evidence {
        Evidence::new(
            "ada".to_owned(),
            "run".to_owned(),
            Url::parse("https://example.com/fact").unwrap(),
            0,
            excerpt,
        )
    }

    pub(crate) fn page<'a>(
        votes: &'a [Vote],
        arguments: &'a [ArgumentView],
        evidence: &'a [Evidence],
        decision: Option<Decision>,
        painted_by: Paint,
    ) -> DayPage<'a> {
        page_named(
            votes,
            arguments,
            evidence,
            decision,
            painted_by,
            "peer-a",
            empty_names(),
        )
    }

    pub(crate) fn page_named<'a>(
        votes: &'a [Vote],
        arguments: &'a [ArgumentView],
        evidence: &'a [Evidence],
        decision: Option<Decision>,
        painted_by: Paint,
        server: &'a str,
        names: &'a BTreeMap<String, String>,
    ) -> DayPage<'a> {
        DayPage {
            day: Day::from_ms(0),
            previous: None,
            next: None,
            votes,
            stances: &[],
            arguments,
            evidence,
            decision,
            chrome: Chrome {
                painted_by,
                as_of_ms: 0,
                rendered_at_ms: 3_723_000,
                server,
                names,
            },
        }
    }

    pub(crate) fn body_open(html: &str) -> &str {
        let start = html.find("<body").expect("body");
        let end = start + html[start..].find('>').expect("body end");
        &html[start..=end]
    }

    pub(crate) fn assert_clean(html: &str) {
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
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::support::{self, assert_clean};
    use super::{ASSETS, render_day, render_front};

    #[test]
    fn assets_match_the_pinned_sha256() {
        let expected = [
            "a79fdb52d4a5c76552452f69202add96e287401fff03d3e8c0e38b4dcb5a99cd",
            "08949f728dc52d528e69b1667d15c89a5686a4ee9a296ff90983985f99c380f7",
            "01d285447409c8a588692162439a038b8cbd7871309ee20267b0d2d91c6e8e22",
            "923f4ddf0fd39f9b7794ab0df7332f3d95dc43e8ad7ec2289d6d9e8491177f51",
            "23b0a9d0c6d3f140a0b77e483c5cfa6bba574325ef5cb189ed9f2fec4884533f",
        ];
        assert_eq!(ASSETS.len(), expected.len());
        for ((path, bytes), hash) in ASSETS.iter().zip(expected) {
            let digest = format!("{:x}", Sha256::digest(bytes));
            assert_eq!(&digest, hash, "{path}");
        }
    }

    #[test]
    fn argument_text_and_excerpt_are_escaped_and_empty_cites_are_named() {
        let arguments = [
            super::ArgumentView {
                id: "arg-2".to_owned(),
                author: "bea".to_owned(),
                at_ms: 1,
                text: "plain".to_owned(),
                cites: Vec::new(),
            },
            super::ArgumentView {
                id: "arg-1".to_owned(),
                author: "ada".to_owned(),
                at_ms: 0,
                text: support::RAW.to_owned(),
                cites: vec![support::evidence(support::RAW)],
            },
        ];
        let shown = support::page(&[], &arguments, &[], None, super::Paint::NoRecord);
        let front = render_front(&shown);
        let day = render_day(&shown);
        for html in [&front, &day] {
            assert!(html.contains(support::ESCAPED));
            assert!(!html.contains(support::RAW));
            assert!(!html.contains("<script"));
            assert_clean(html);
        }
        // The day page still lists every argument, oldest first, including the
        // cite URL. Today keeps the escaped text and drops that wording.
        assert!(day.contains("plain"));
        assert!(day.contains("cites no evidence"));
        assert!(day.contains("https://example.com/fact"));
        let raw_at = day.find("arg-1").expect("earlier argument id");
        let plain_at = day.find("arg-2").expect("later argument id");
        assert!(raw_at < plain_at);
        assert!(front.contains("argued, not voted yet"));
    }
}
