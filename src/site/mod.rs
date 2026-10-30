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
        assert!(front.contains("argued, voting"));
        assert!(front.contains("no vote yet"));
    }
}
