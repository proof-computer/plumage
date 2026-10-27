//! Exact counts over a day's vote, reply and evidence records.
//!
//! Each count is a pure function. The formula beside it is the published rule.

use std::collections::{BTreeMap, BTreeSet};

use crate::closing::Vote;
use crate::colour::Colour;
use crate::evidence::{Evidence, EvidenceId};

/// One UTC hour, in milliseconds. The curve samples on this grid.
const HOUR_MS: u64 = 3_600_000;

pub const CURRENT_VOTES_RULE: &str = "An author's current vote is that author's newest vote at or before the time. Votes sharing a time are ordered by (at_ms, author, colour), and the last of those is current.";
pub const COUNTS_RULE: &str =
    "Each colour's count is the number of authors whose current vote is that colour.";
pub const VOTE_CHANGES_RULE: &str = "A vote change is a vote whose colour differs from that author's previous vote. The first vote and a repeat of the same colour are not changes.";
pub const REPLIES_RULE: &str = "A reply is an argument that names another author's argument as reply_to. The day's reply count is the number of such arguments.";
pub const REPLY_LED_CHANGES_RULE: &str = "A reply-led change is a vote change (see the vote change rule) by an author who received at least one reply after that author's previous vote and at or before the changing vote. Each change counts once, however many replies came before it.";
pub const CURVE_RULE: &str = "The curve is the colour counts at every whole hour from one hour after the day's open through the day's close.";
pub const FETCHES_RULE: &str =
    "An author's fetch count is the number of evidence records that author stored.";
pub const VOTES_CAST_RULE: &str =
    "Votes cast is the number of vote records from the day's open through the time.";
pub const ARGUMENTS_RULE: &str = "An argument count is the number of argue records an author wrote; the day's total is their sum.";
pub const TRAIL_RULE: &str = "An author's trail is that author's votes in the order the closing rule replays them, oldest first.";
pub const CITATIONS_RULE: &str = "A fact's citation count is the number of arguments that cite it; an argument citing it twice counts once.";
pub const DAYS_PER_COLOUR_RULE: &str = "A colour's day count is the number of closed days whose decision named it. Undecided days are counted apart.";

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

/// One reply on the day clock: the replying argument's time, and the author of
/// the argument it answers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub at_ms: u64,
    pub target_author: String,
}

/// How many replies the day holds. See [`REPLIES_RULE`].
pub fn replies(replies: &[Reply]) -> usize {
    replies.len()
}

/// Reply-led vote changes. See [`REPLY_LED_CHANGES_RULE`].
///
/// A vote counts when it differs from that author's previous vote in the same
/// `(at_ms, author, colour)` order [`vote_changes`] and [`crate::closing::decide`]
/// use, and some reply to that author sits strictly after the previous vote
/// and at or before the changing vote.
pub fn reply_led_changes(votes: &[Vote], replies: &[Reply]) -> usize {
    let mut ordered: Vec<&Vote> = votes.iter().collect();
    ordered.sort_by_key(|vote| (vote.at_ms, vote.author.as_str(), vote.colour));
    let mut previous: BTreeMap<&str, (u64, Colour)> = BTreeMap::new();
    let mut count = 0;
    for vote in ordered {
        if let Some(&(prior_at, prior_colour)) = previous.get(vote.author.as_str())
            && prior_colour != vote.colour
            && replies.iter().any(|reply| {
                reply.target_author == vote.author
                    && prior_at < reply.at_ms
                    && reply.at_ms <= vote.at_ms
            })
        {
            count += 1;
        }
        previous.insert(vote.author.as_str(), (vote.at_ms, vote.colour));
    }
    count
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

/// Vote records from `open_ms` through `at_ms`, both ends included.
pub fn votes_cast(votes: &[Vote], open_ms: u64, at_ms: u64) -> usize {
    votes
        .iter()
        .filter(|vote| open_ms <= vote.at_ms && vote.at_ms <= at_ms)
        .count()
}

/// Each author's votes in the order [`crate::closing::decide`] replays them.
///
/// Authors who cast no vote are absent.
pub fn trails(votes: &[Vote]) -> BTreeMap<String, Vec<(u64, Colour)>> {
    let mut ordered: Vec<&Vote> = votes.iter().collect();
    ordered.sort_by_key(|vote| (vote.at_ms, vote.author.as_str(), vote.colour));
    let mut trails: BTreeMap<String, Vec<(u64, Colour)>> = BTreeMap::new();
    for vote in ordered {
        trails
            .entry(vote.author.clone())
            .or_default()
            .push((vote.at_ms, vote.colour));
    }
    trails
}

/// Arguments citing each evidence id. An id cited twice by one argument counts once.
///
/// An id no argument cites is absent. The key is the id's string.
pub fn citations<'a, A, C>(arguments: A) -> BTreeMap<String, usize>
where
    A: IntoIterator<Item = C>,
    C: IntoIterator<Item = &'a EvidenceId>,
{
    let mut counts = BTreeMap::new();
    for argument in arguments {
        let mut seen = BTreeSet::new();
        for id in argument {
            if seen.insert(id.0.as_str()) {
                *counts.entry(id.0.clone()).or_default() += 1;
            }
        }
    }
    counts
}

/// Closed days per colour, and the number of undecided days.
///
/// `None` is an undecided day. A colour no decision named is absent.
pub fn days_per_colour(
    decisions: impl IntoIterator<Item = Option<Colour>>,
) -> (BTreeMap<Colour, usize>, usize) {
    let mut days = BTreeMap::new();
    let mut undecided = 0;
    for decision in decisions {
        match decision {
            Some(colour) => *days.entry(colour).or_default() += 1,
            None => undecided += 1,
        }
    }
    (days, undecided)
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

    fn reply(at_ms: u64, target_author: &str) -> Reply {
        Reply {
            at_ms,
            target_author: target_author.to_owned(),
        }
    }

    #[test]
    fn replies_count_each_reply_once() {
        assert_eq!(replies(&[]), 0);
        assert_eq!(replies(&[reply(1, "b"), reply(2, "b"), reply(3, "c")]), 3);
    }

    #[test]
    fn reply_led_changes_follow_the_published_window() {
        let changed = [vote("b", Blue, 1), vote("b", Red, 3)];
        // b votes blue, a replies to b, then b votes red.
        assert_eq!(reply_led_changes(&changed, &[reply(2, "b")]), 1);
        // The same votes with b voting blue again are not a change.
        let repeated = [vote("b", Blue, 1), vote("b", Blue, 3)];
        assert_eq!(reply_led_changes(&repeated, &[reply(2, "b")]), 0);
        // A change with no reply since b's previous vote.
        assert_eq!(reply_led_changes(&changed, &[]), 0);
        assert_eq!(reply_led_changes(&changed, &[reply(0, "b")]), 0);
        // A reply before b's first vote, then that first vote.
        assert_eq!(
            reply_led_changes(&[vote("b", Blue, 2)], &[reply(1, "b")]),
            0
        );
        // Two replies before one change count once.
        assert_eq!(
            reply_led_changes(&changed, &[reply(2, "b"), reply(2, "b")]),
            1
        );
        // A reply at exactly the previous vote does not count; one at the
        // changing vote does.
        assert_eq!(reply_led_changes(&changed, &[reply(1, "b")]), 0);
        assert_eq!(reply_led_changes(&changed, &[reply(3, "b")]), 1);
        // A reply to c does not count for b.
        assert_eq!(reply_led_changes(&changed, &[reply(2, "c")]), 0);
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

    #[test]
    fn votes_cast_counts_only_the_closed_window() {
        let votes = [
            vote("ada", Red, 0),
            vote("bea", Blue, 5),
            vote("cy", Red, 10),
        ];
        assert_eq!(votes_cast(&votes, 0, 10), 3);
        assert_eq!(votes_cast(&votes, 5, 5), 1);
        assert_eq!(votes_cast(&votes, 6, 9), 0);
        assert_eq!(votes_cast(&votes, 11, 10), 0);
    }

    #[test]
    fn trails_order_same_time_votes_as_decide_does() {
        let tied = [
            vote("ada", Blue, 5),
            vote("ada", Red, 5),
            vote("bea", Red, 5),
        ];
        assert_eq!(
            trails(&tied).get("ada").map(Vec::as_slice),
            Some([(5, Red), (5, Blue)].as_slice())
        );
        assert_eq!(
            current_votes(&tied, 5).get("ada"),
            trails(&tied)
                .get("ada")
                .and_then(|trail| trail.last())
                .map(|(_, colour)| colour)
        );
    }

    #[test]
    fn citations_count_an_argument_once_per_fact() {
        let cited = EvidenceId("cited".to_owned());
        let other = EvidenceId("other".to_owned());
        let arguments = [vec![&cited, &cited, &other], vec![&cited], vec![]];
        assert_eq!(
            citations(arguments),
            BTreeMap::from([("cited".to_owned(), 2), ("other".to_owned(), 1)])
        );
        assert!(citations(Vec::<Vec<&EvidenceId>>::new()).is_empty());
    }

    #[test]
    fn days_per_colour_counts_decided_days_and_undecided_apart() {
        assert_eq!(
            days_per_colour([Some(Blue), None, Some(Blue), Some(Red)]),
            (BTreeMap::from([(Blue, 2), (Red, 1)]), 1)
        );
    }
}
