//! Exact counts over a day's vote and evidence records.
//!
//! Each count is a pure function. The formula beside it is the published rule.

use std::collections::BTreeMap;

use crate::closing::Vote;
use crate::colour::Colour;
use crate::evidence::Evidence;

/// One UTC hour, in milliseconds. The curve samples on this grid.
const HOUR_MS: u64 = 3_600_000;

pub const CURRENT_VOTES_RULE: &str = "An author's current vote is that author's newest vote at or before the time. Votes sharing a time are ordered by (at_ms, author, colour), and the last of those is current.";
pub const COUNTS_RULE: &str =
    "Each colour's count is the number of authors whose current vote is that colour.";
pub const VOTE_CHANGES_RULE: &str = "A vote change is a vote whose colour differs from that author's previous vote. The first vote and a repeat of the same colour are not changes.";
pub const CURVE_RULE: &str = "The curve is the colour counts at every whole hour from one hour after the day's open through the day's close.";
pub const FETCHES_RULE: &str =
    "An author's fetch count is the number of evidence records that author stored.";

/// Each author's current colour at `at_ms`.
///
/// Authors with no vote at or before `at_ms` are absent. The order of ties is
/// the same `(at_ms, author, colour)` order `closing::decide` replays.
pub fn current_votes(votes: &[Vote], at_ms: u64) -> BTreeMap<String, Colour> {
    let mut ordered: Vec<&Vote> = votes.iter().filter(|vote| vote.at_ms <= at_ms).collect();
    ordered.sort_by_key(|vote| (vote.at_ms, vote.author.as_str(), vote.colour));
    let mut current = BTreeMap::new();
    for vote in ordered {
        current.insert(vote.author.clone(), vote.colour);
    }
    current
}

/// Authors per colour at `at_ms`. Colours nobody currently holds are absent.
pub fn counts(votes: &[Vote], at_ms: u64) -> BTreeMap<Colour, usize> {
    let mut counts = BTreeMap::new();
    for colour in current_votes(votes, at_ms).values() {
        *counts.entry(*colour).or_default() += 1;
    }
    counts
}

/// Vote changes per author.
///
/// Every author who cast a vote is present. An author who never changed colour
/// maps to zero.
pub fn vote_changes(votes: &[Vote]) -> BTreeMap<String, usize> {
    let mut ordered: Vec<&Vote> = votes.iter().collect();
    ordered.sort_by_key(|vote| (vote.at_ms, vote.author.as_str(), vote.colour));
    let mut previous: BTreeMap<&str, Colour> = BTreeMap::new();
    let mut changes: BTreeMap<String, usize> = BTreeMap::new();
    for vote in ordered {
        let entry = changes.entry(vote.author.clone()).or_default();
        if previous
            .get(vote.author.as_str())
            .is_some_and(|colour| *colour != vote.colour)
        {
            *entry += 1;
        }
        previous.insert(vote.author.as_str(), vote.colour);
    }
    changes
}

/// `counts` at each whole hour from `open_ms + 1h` through `close_ms`.
///
/// A window shorter than one hour, or an `open_ms` so large that one hour
/// does not fit in `u64`, is an empty curve.
pub fn curve(votes: &[Vote], open_ms: u64, close_ms: u64) -> Vec<(u64, BTreeMap<Colour, usize>)> {
    let mut points = Vec::new();
    let mut at = match open_ms.checked_add(HOUR_MS) {
        Some(at) => at,
        None => return points,
    };
    while at <= close_ms {
        points.push((at, counts(votes, at)));
        match at.checked_add(HOUR_MS) {
            Some(next) => at = next,
            None => break,
        }
    }
    points
}

/// Evidence records per author. Authors with no evidence are absent.
pub fn fetches(evidence: &[Evidence]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for record in evidence {
        *counts.entry(record.author().to_owned()).or_default() += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use url::Url;

    use super::*;
    use crate::colour::Colour::{Blue, Red};
    use crate::evidence::Evidence;

    fn vote(author: &str, colour: Colour, at_ms: u64) -> Vote {
        Vote {
            author: author.to_owned(),
            colour,
            at_ms,
        }
    }

    fn fixed_votes() -> Vec<Vote> {
        vec![
            vote("ada", Red, 0),
            vote("ada", Red, 30 * 60 * 1000),
            vote("ada", Blue, HOUR_MS),
            vote("bea", Blue, 0),
            vote("bea", Blue, HOUR_MS),
            vote("cy", Red, 23 * HOUR_MS),
        ]
    }

    #[test]
    fn current_vote_is_the_last_in_decide_order() {
        let tied = [vote("ada", Blue, 5), vote("ada", Red, 5)];
        assert_eq!(current_votes(&tied, 5).get("ada"), Some(&Blue));
        assert!(current_votes(&tied, 4).is_empty());

        let votes = fixed_votes();
        assert_eq!(
            current_votes(&votes, 0),
            BTreeMap::from([("ada".to_owned(), Red), ("bea".to_owned(), Blue)])
        );
        assert_eq!(
            current_votes(&votes, HOUR_MS),
            BTreeMap::from([("ada".to_owned(), Blue), ("bea".to_owned(), Blue)])
        );
    }

    #[test]
    fn counts_vote_changes_and_curve_match_the_fixed_list() {
        let votes = fixed_votes();
        assert_eq!(counts(&votes, 0), BTreeMap::from([(Red, 1), (Blue, 1)]));
        assert_eq!(counts(&votes, HOUR_MS), BTreeMap::from([(Blue, 2)]));
        assert_eq!(
            vote_changes(&votes),
            BTreeMap::from([
                ("ada".to_owned(), 1),
                ("bea".to_owned(), 0),
                ("cy".to_owned(), 0),
            ])
        );

        let close = 23 * HOUR_MS;
        let points = curve(&votes, 0, close);
        assert_eq!(points.len(), 23);
        assert_eq!(points[0].0, HOUR_MS);
        assert_eq!(points[0].1, BTreeMap::from([(Blue, 2)]));
        assert_eq!(points[22].0, close);
        assert_eq!(points[22].1, BTreeMap::from([(Blue, 2), (Red, 1)]));
        assert!(curve(&votes, close, HOUR_MS).is_empty());
    }

    #[test]
    fn fetches_count_evidence_records_per_author() {
        let url = Url::parse("https://example.com/fact").unwrap();
        let evidence = [
            Evidence::new("ada".to_owned(), "run".to_owned(), url.clone(), 0, "one"),
            Evidence::new("ada".to_owned(), "run".to_owned(), url.clone(), 1, "two"),
            Evidence::new("bea".to_owned(), "run".to_owned(), url, 2, "three"),
        ];
        assert_eq!(
            fetches(&evidence),
            BTreeMap::from([("ada".to_owned(), 2), ("bea".to_owned(), 1)])
        );
        assert!(fetches(&[]).is_empty());
    }
}
