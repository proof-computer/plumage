//! Machine-readable status of one peer.
//!
//! Pure: no file, network, store or clock. The caller supplies every time
//! and every record. The file carries counts, ids, colour names, error codes
//! and times. Argument text, evidence excerpts and URLs stay with the caller.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;

use super::Paint;
use crate::closing::{self, Vote};
use crate::colour::Colour;
use crate::day::Day;
use crate::evidence::Evidence;
use crate::tally;

/// Schema name published in every status file.
pub const SCHEMA: &str = "plumage.status.v1";

/// Largest status file the builder may emit, including its trailing newline.
pub const MAX_STATUS_BYTES: usize = 16_384;

/// `authors_seen` keeps at most this many ids. Further ids set `authors_truncated`.
pub const MAX_AUTHORS: usize = 256;

/// How many of this peer's latest activations `recent` remembers.
pub const RECENT_WINDOW: usize = 20;

/// Where the peer is in the day.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Activations are still running.
    Voting,
    /// The day has reached close and the peer is waiting to publish.
    SyncOnly,
    /// The day's decision is published.
    Closed,
}

/// Activation counts since this process started. They are not persisted.
#[derive(Clone, Debug, Default)]
pub struct Activations {
    /// Activations that returned, vote or error.
    pub started: u64,
    /// Activations that ended in a vote.
    pub voted: u64,
    /// Errors by the `code` string of [`crate::error::Error`].
    pub errors_by_code: BTreeMap<&'static str, u64>,
    /// Failed fetches by [`crate::fetch::FetchError`] code, since process start.
    pub fetch_failures_by_code: BTreeMap<&'static str, u64>,
    /// Wall-clock time of the latest activation, when one has finished.
    pub last_real_ms: Option<u64>,
    /// `voted:<colour>` or `error:<code>` for the latest activation.
    pub last_outcome: Option<String>,
    /// `true` when that activation ended in an error. Oldest dropped first.
    recent: VecDeque<bool>,
}

impl Activations {
    /// One activation that voted `colour` at wall-clock `real_ms`.
    pub fn record_vote(&mut self, colour: Colour, real_ms: u64) {
        self.started += 1;
        self.voted += 1;
        self.last_real_ms = Some(real_ms);
        self.last_outcome = Some(format!("voted:{}", colour.name()));
        self.push_recent(false);
    }

    /// One activation that failed with `code` at wall-clock `real_ms`.
    pub fn record_error(&mut self, code: &'static str, real_ms: u64) {
        self.started += 1;
        *self.errors_by_code.entry(code).or_default() += 1;
        self.last_real_ms = Some(real_ms);
        self.last_outcome = Some(format!("error:{code}"));
        self.push_recent(true);
    }

    /// One fetch that returned `code`. It does not count as an activation.
    pub fn record_fetch_failure(&mut self, code: &'static str) {
        *self.fetch_failures_by_code.entry(code).or_default() += 1;
    }

    fn push_recent(&mut self, error: bool) {
        if self.recent.len() == RECENT_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(error);
    }
}

/// Everything [`render_status`] needs. `arguments` is a count: the text stays out.
pub struct StatusInput<'a> {
    /// This peer's id.
    pub peer_id: &'a str,
    /// `CARGO_PKG_VERSION` of the binary that is writing.
    pub version: &'a str,
    /// `LISKOV_PLUMAGE_BUILD_COMMIT` when the binary was built with one.
    pub commit: Option<&'a str>,
    /// Wall-clock time at process start.
    pub started_real_ms: u64,
    /// Wall-clock time at this write.
    pub as_of_ms: u64,
    /// Day-clock time at this write.
    pub as_of_sim_ms: u64,
    /// The day this snapshot describes.
    pub day: Day,
    /// Phase of `day`.
    pub phase: Phase,
    /// Votes on the day clock.
    pub votes: &'a [Vote],
    /// Evidence records. Only their authors and the count are read.
    pub evidence: &'a [Evidence],
    /// Argument records in the day's book. The text is not an input.
    pub arguments: usize,
    /// Replies on the day clock. The file stores the count, not the arguments.
    pub replies: &'a [tally::Reply],
    /// Distinct authors of votes, arguments and evidence, sorted by the caller.
    pub authors_seen: &'a BTreeSet<String>,
    /// Counts since process start.
    pub activations: &'a Activations,
    /// The previous painted day, or at close the day just decided.
    pub previous_day: (Day, Paint),
}

/// Pretty JSON, then a newline.
pub fn render_status(input: &StatusInput<'_>) -> Vec<u8> {
    let until = input.as_of_sim_ms.min(input.day.close_ms());
    let mut authors_seen: Vec<&str> = input.authors_seen.iter().map(String::as_str).collect();
    let authors_truncated = authors_seen.len() > MAX_AUTHORS;
    authors_seen.truncate(MAX_AUTHORS);
    let counts = tally::counts(input.votes, input.as_of_sim_ms)
        .into_iter()
        .map(|(colour, count)| (colour.name(), count))
        .collect::<BTreeMap<_, _>>();
    let recent_errors = input
        .activations
        .recent
        .iter()
        .filter(|error| **error)
        .count();
    let document = Document {
        schema: SCHEMA,
        peer_id: input.peer_id,
        build: Build {
            version: input.version,
            commit: input.commit,
        },
        started_real_ms: input.started_real_ms,
        as_of_ms: input.as_of_ms,
        as_of_sim_ms: input.as_of_sim_ms,
        day: input.day,
        day_open_ms: input.day.open_ms(),
        phase: input.phase,
        live_voters: closing::live_voters(input.votes, input.day.open_ms(), until),
        counts,
        records: Records {
            votes: input.votes.len(),
            arguments: input.arguments,
            evidence: input.evidence.len(),
            replies: tally::replies(input.replies),
        },
        authors_seen,
        authors_truncated,
        fetches: tally::fetches(input.evidence)
            .get(input.peer_id)
            .copied()
            .unwrap_or(0),
        reply_led_changes: tally::reply_led_changes(input.votes, input.replies),
        fetch_failures_by_code: &input.activations.fetch_failures_by_code,
        activations: ActivationsView {
            started: input.activations.started,
            voted: input.activations.voted,
            errors_by_code: &input.activations.errors_by_code,
            last_real_ms: input.activations.last_real_ms,
            last_outcome: input.activations.last_outcome.as_deref(),
            recent: RecentView {
                window: RECENT_WINDOW,
                total: input.activations.recent.len(),
                errors: recent_errors,
            },
        },
        previous_day: PreviousDay {
            day: input.previous_day.0,
            paint: paint_name(input.previous_day.1),
        },
    };
    let mut bytes = serde_json::to_vec_pretty(&document).expect("status serializes");
    bytes.push(b'\n');
    bytes
}

fn paint_name(paint: Paint) -> String {
    match paint {
        Paint::Decided(colour) => format!("decided:{}", colour.name()),
        Paint::Undecided => "undecided".to_owned(),
        Paint::NoRecord => "no_record".to_owned(),
    }
}

#[derive(Serialize)]
struct Document<'a> {
    schema: &'a str,
    peer_id: &'a str,
    build: Build<'a>,
    started_real_ms: u64,
    as_of_ms: u64,
    as_of_sim_ms: u64,
    day: Day,
    day_open_ms: u64,
    phase: Phase,
    live_voters: Vec<String>,
    counts: BTreeMap<&'a str, usize>,
    records: Records,
    authors_seen: Vec<&'a str>,
    authors_truncated: bool,
    fetches: usize,
    reply_led_changes: usize,
    fetch_failures_by_code: &'a BTreeMap<&'static str, u64>,
    activations: ActivationsView<'a>,
    previous_day: PreviousDay,
}

#[derive(Serialize)]
struct Build<'a> {
    version: &'a str,
    commit: Option<&'a str>,
}

#[derive(Serialize)]
struct Records {
    votes: usize,
    arguments: usize,
    evidence: usize,
    replies: usize,
}

#[derive(Serialize)]
struct ActivationsView<'a> {
    started: u64,
    voted: u64,
    errors_by_code: &'a BTreeMap<&'static str, u64>,
    last_real_ms: Option<u64>,
    last_outcome: Option<&'a str>,
    recent: RecentView,
}

#[derive(Serialize)]
struct RecentView {
    window: usize,
    total: usize,
    errors: usize,
}

#[derive(Serialize)]
struct PreviousDay {
    day: Day,
    paint: String,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::{Value, json};
    use url::Url;

    use super::*;

    const SECRET: &str = "SECRET-ARGUMENT-TEXT";

    /// Codes plumage itself names on [`crate::error::Error`]. Upstream crates
    /// add more; this is the set this repository spells out.
    const ERROR_CODES: &[&str] = &[
        "budget_exhausted",
        "config",
        "conflict",
        "fetch_failed",
        "fetch_status",
        "invalid",
        "invalid_action",
        "limit",
        "missing_reference",
        "not_allowed",
        "refused",
        "source_invalid",
        "storage",
        "too_large",
        "tool_limit",
    ];

    struct Fixture {
        day: Day,
        phase: Phase,
        peer_id: String,
        votes: Vec<Vote>,
        evidence: Vec<Evidence>,
        arguments: usize,
        replies: Vec<tally::Reply>,
        authors: BTreeSet<String>,
        activations: Activations,
        previous: (Day, Paint),
        as_of_ms: u64,
        as_of_sim_ms: u64,
        started_real_ms: u64,
    }

    impl Fixture {
        fn on(day: Day) -> Self {
            let open = day.open_ms();
            Self {
                day,
                phase: Phase::Voting,
                peer_id: "p00".to_owned(),
                votes: Vec::new(),
                evidence: Vec::new(),
                arguments: 0,
                replies: Vec::new(),
                authors: BTreeSet::new(),
                activations: Activations::default(),
                previous: (day.previous(), Paint::NoRecord),
                as_of_ms: 0,
                as_of_sim_ms: open,
                started_real_ms: 0,
            }
        }

        fn render(&self) -> Vec<u8> {
            render_status(&StatusInput {
                peer_id: &self.peer_id,
                version: "0.1.0",
                commit: None,
                started_real_ms: self.started_real_ms,
                as_of_ms: self.as_of_ms,
                as_of_sim_ms: self.as_of_sim_ms,
                day: self.day,
                phase: self.phase,
                votes: &self.votes,
                evidence: &self.evidence,
                arguments: self.arguments,
                replies: &self.replies,
                authors_seen: &self.authors,
                activations: &self.activations,
                previous_day: self.previous,
            })
        }
    }

    fn day() -> Day {
        Day::from_str("2026-10-26").unwrap()
    }

    fn parse(bytes: &[u8]) -> Value {
        assert!(bytes.ends_with(b"\n"));
        serde_json::from_slice(bytes).unwrap()
    }

    fn vote(author: &str, colour: Colour, at_ms: u64) -> Vote {
        Vote {
            author: author.to_owned(),
            colour,
            at_ms,
        }
    }

    #[test]
    fn fixture_day_renders_the_published_fields() {
        let open = day().open_ms();
        assert_eq!(open, 1_792_972_800_000);
        let mut activations = Activations::default();
        activations.record_vote(Colour::Blue, 0);
        let mut fixture = Fixture::on(day());
        fixture.votes = vec![vote("p00", Colour::Blue, open)];
        fixture.arguments = 1;
        fixture.authors.insert("p00".to_owned());
        fixture.activations = activations;
        // The vote sits at the open. An `as_of_sim_ms` of 0, as in the
        // shape example, is before the day and would list no live voter.
        fixture.as_of_sim_ms = open;

        let value = parse(&fixture.render());
        assert_eq!(
            value,
            json!({
                "schema": "plumage.status.v1",
                "peer_id": "p00",
                "build": {"version": "0.1.0", "commit": null},
                "started_real_ms": 0,
                "as_of_ms": 0,
                "as_of_sim_ms": open,
                "day": "2026-10-26",
                "day_open_ms": open,
                "phase": "voting",
                "live_voters": ["p00"],
                "counts": {"blue": 1},
                "records": {"votes": 1, "arguments": 1, "evidence": 0, "replies": 0},
                "authors_seen": ["p00"],
                "authors_truncated": false,
                "fetches": 0,
                "reply_led_changes": 0,
                "fetch_failures_by_code": {},
                "activations": {
                    "started": 1,
                    "voted": 1,
                    "errors_by_code": {},
                    "last_real_ms": 0,
                    "last_outcome": "voted:blue",
                    "recent": {"window": 20, "total": 1, "errors": 0}
                },
                "previous_day": {"day": "2026-10-25", "paint": "no_record"}
            })
        );

        for (phase, name) in [
            (Phase::Voting, "voting"),
            (Phase::SyncOnly, "sync_only"),
            (Phase::Closed, "closed"),
        ] {
            fixture.phase = phase;
            assert_eq!(parse(&fixture.render())["phase"], name);
        }
        for (paint, name) in [
            (Paint::Decided(Colour::Blue), "decided:blue"),
            (Paint::Undecided, "undecided"),
            (Paint::NoRecord, "no_record"),
        ] {
            fixture.previous = (day(), paint);
            assert_eq!(parse(&fixture.render())["previous_day"]["paint"], name);
        }
    }

    #[test]
    fn replies_and_reply_led_changes_are_the_published_counts() {
        let open = day().open_ms();
        let mut fixture = Fixture::on(day());
        fixture.votes = vec![
            vote("b", Colour::Blue, open + 1),
            vote("b", Colour::Red, open + 3),
        ];
        fixture.replies = vec![tally::Reply {
            at_ms: open + 2,
            target_author: "b".to_owned(),
        }];
        let value = parse(&fixture.render());
        assert_eq!(value["records"]["replies"], 1);
        assert_eq!(value["reply_led_changes"], 1);
    }

    #[test]
    fn recent_window_keeps_the_last_twenty_and_a_shorter_run_stays_short() {
        let mut activations = Activations::default();
        for _ in 0..22 {
            activations.record_vote(Colour::Blue, 0);
        }
        activations.record_error("storage", 1);
        activations.record_error("storage", 2);
        activations.record_error("config", 3);
        let mut fixture = Fixture::on(day());
        fixture.activations = activations;
        assert_eq!(
            parse(&fixture.render())["activations"]["recent"],
            json!({"window": 20, "total": 20, "errors": 3})
        );

        let mut five = Activations::default();
        for _ in 0..5 {
            five.record_vote(Colour::Red, 0);
        }
        fixture.activations = five;
        assert_eq!(
            parse(&fixture.render())["activations"]["recent"],
            json!({"window": 20, "total": 5, "errors": 0})
        );
    }

    #[test]
    fn argument_text_and_evidence_body_stay_out_of_the_file() {
        let url = Url::parse(&format!("https://example.invalid/{SECRET}")).unwrap();
        let evidence = Evidence::new("p00".to_owned(), "2026-10-26".to_owned(), url, 0, SECRET);
        let mut fixture = Fixture::on(day());
        // One argument whose text is `SECRET`. The builder takes the count only.
        // A reply names that same string as its target author and still leaves
        // it out of the file.
        fixture.arguments = 1;
        fixture.evidence = vec![evidence];
        fixture.replies = vec![tally::Reply {
            at_ms: 0,
            target_author: SECRET.to_owned(),
        }];
        let rendered = String::from_utf8(fixture.render()).unwrap();
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains("example.invalid"));
        assert_eq!(parse(rendered.as_bytes())["records"]["arguments"], 1);
        assert_eq!(parse(rendered.as_bytes())["records"]["replies"], 1);
        assert_eq!(parse(rendered.as_bytes())["fetches"], 1);
    }

    #[test]
    fn full_vocabulary_stays_within_the_byte_bound() {
        let open = day().open_ms();
        let mut authors = BTreeSet::new();
        for index in 0..300 {
            authors.insert(format!("p{index:03}"));
        }
        let votes = Colour::ALL
            .into_iter()
            .enumerate()
            .map(|(index, colour)| vote(&format!("p{index:03}"), colour, open))
            .collect();
        let mut activations = Activations::default();
        for code in ERROR_CODES {
            activations.record_error(code, 0);
        }
        let mut fixture = Fixture::on(day());
        fixture.votes = votes;
        fixture.arguments = 300;
        fixture.replies = (0..256)
            .map(|at_ms| tally::Reply {
                at_ms,
                target_author: format!("p{at_ms:03}"),
            })
            .collect();
        fixture.authors = authors;
        fixture.activations = activations;
        fixture.as_of_sim_ms = open;
        let bytes = fixture.render();
        assert!(
            bytes.len() <= MAX_STATUS_BYTES,
            "status is {} bytes",
            bytes.len()
        );
        let value = parse(&bytes);
        assert_eq!(value["records"]["replies"], 256);
        assert_eq!(value["reply_led_changes"], 0);
        assert_eq!(value["authors_truncated"], true);
        let seen = value["authors_seen"].as_array().unwrap();
        assert_eq!(seen.len(), MAX_AUTHORS);
        assert_eq!(seen[0], "p000");
        assert_eq!(seen[MAX_AUTHORS - 1], "p255");
        assert!(!seen.iter().any(|author| author == "p256"));
        assert_eq!(
            value["counts"].as_object().unwrap().len(),
            Colour::ALL.len()
        );
        for colour in Colour::ALL {
            assert!(value["counts"].get(colour.name()).is_some());
        }
        for code in ERROR_CODES {
            assert_eq!(value["activations"]["errors_by_code"][code], 1);
        }
    }

    #[test]
    fn live_voters_ignore_votes_after_as_of_and_after_close() {
        let open = day().open_ms();
        let close = day().close_ms();
        let as_of = open + 60_000;
        let mut fixture = Fixture::on(day());
        fixture.as_of_sim_ms = as_of;
        fixture.votes = vec![
            vote("open", Colour::Blue, open),
            vote("as-of", Colour::Blue, as_of),
            vote("after-as-of", Colour::Red, as_of + 1),
            vote("close", Colour::Red, close),
            vote("after-close", Colour::Red, close + 1),
        ];
        assert_eq!(
            parse(&fixture.render())["live_voters"],
            json!(["as-of", "open"])
        );

        fixture.as_of_sim_ms = close + 60_000;
        fixture.votes = vec![
            vote("open", Colour::Blue, open),
            vote("close", Colour::Blue, close),
            vote("after-close", Colour::Red, close + 1),
        ];
        assert_eq!(
            parse(&fixture.render())["live_voters"],
            json!(["close", "open"])
        );
    }

    #[test]
    fn fetch_failures_render_by_code() {
        let mut activations = Activations::default();
        activations.record_fetch_failure("fetch_status");
        activations.record_fetch_failure("fetch_status");
        activations.record_fetch_failure("too_large");
        let mut fixture = Fixture::on(day());
        fixture.activations = activations;
        assert_eq!(
            parse(&fixture.render())["fetch_failures_by_code"],
            json!({"fetch_status": 2, "too_large": 1})
        );
    }

    #[test]
    fn every_fetch_failure_code_at_max_stays_within_the_byte_bound() {
        let mut activations = Activations::default();
        for code in [
            "not_allowed",
            "fetch_failed",
            "fetch_status",
            "too_large",
            "source_invalid",
        ] {
            activations.fetch_failures_by_code.insert(code, u64::MAX);
        }
        let mut fixture = Fixture::on(day());
        fixture.activations = activations;
        let bytes = fixture.render();
        assert!(
            bytes.len() <= MAX_STATUS_BYTES,
            "status is {} bytes",
            bytes.len()
        );
        let value = parse(&bytes);
        for code in [
            "not_allowed",
            "fetch_failed",
            "fetch_status",
            "too_large",
            "source_invalid",
        ] {
            assert_eq!(value["fetch_failures_by_code"][code], u64::MAX);
        }
    }
}
