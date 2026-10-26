//! The day's closing rule over caller-supplied live voters and vote records.

use crate::colour::Colour;
use std::collections::{BTreeMap, BTreeSet};

pub const HOLD_MS: u64 = 3_600_000;
pub const RULE: &str = "The day's colour is the first colour held by more than half of the live voters for a continuous hour ending at or before close; otherwise the day is undecided.";
pub const LIVE_RULE: &str = "The live voters are the distinct authors of vote records cast between the day's open (00:00 UTC) and close (23:00 UTC).";
pub const HOLDING_RULE: &str = "A colour holds while more than half of the live voters' current votes name it. A hold starts at the vote that made it more than half and ends at the vote that made it half or fewer.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vote {
    pub author: String,
    pub colour: Colour,
    pub at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Decided {
        colour: Colour,
        held_from_ms: u64,
        held_to_ms: u64,
    },
    Undecided,
}

/// One run in which a colour held more than half of the live voters.
///
/// `to_ms` is the vote that ended the run. It is absent while the run is still
/// held after the last replayed vote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hold {
    pub colour: Colour,
    pub from_ms: u64,
    pub to_ms: Option<u64>,
}

/// Replays in `(at_ms, author, colour)` order, using `Colour`'s declaration order
/// for simultaneous votes by the same author. The last such vote is current.
///
/// The replay is [`holds`]. The decision is the first run that lasted [`HOLD_MS`].
pub fn decide(votes: &[Vote], live: &[String], open_ms: u64, close_ms: u64) -> Decision {
    holds(votes, live, open_ms, close_ms)
        .into_iter()
        .find_map(|hold| decision_for(hold, close_ms))
        .unwrap_or(Decision::Undecided)
}

/// Every majority run, in replay order. The filter, sort, majority test and
/// first-colour choice are the ones [`decide`] uses.
pub fn holds(votes: &[Vote], live: &[String], open_ms: u64, until_ms: u64) -> Vec<Hold> {
    let authors: BTreeSet<_> = live.iter().map(String::as_str).collect();
    let mut votes: Vec<_> = votes
        .iter()
        .filter(|v| {
            authors.contains(v.author.as_str()) && open_ms <= v.at_ms && v.at_ms <= until_ms
        })
        .collect();
    votes.sort_by_key(|v| (v.at_ms, v.author.as_str(), v.colour));

    let mut current = BTreeMap::new();
    let mut counts: BTreeMap<Colour, usize> = BTreeMap::new();
    let mut runs: Vec<Hold> = Vec::new();
    for vote in votes {
        if let Some(previous) = current.insert(vote.author.as_str(), vote.colour) {
            *counts.entry(previous).or_default() -= 1;
        }
        *counts.entry(vote.colour).or_default() += 1;
        let majority = counts
            .iter()
            .find_map(|(&colour, &count)| (count > live.len() / 2).then_some(colour));
        let held_colour = runs
            .last()
            .filter(|hold| hold.to_ms.is_none())
            .map(|hold| hold.colour);
        if majority != held_colour {
            if let Some(open) = runs.last_mut().filter(|hold| hold.to_ms.is_none()) {
                open.to_ms = Some(vote.at_ms);
            }
            if let Some(colour) = majority {
                runs.push(Hold {
                    colour,
                    from_ms: vote.at_ms,
                    to_ms: None,
                });
            }
        }
    }
    runs
}

/// The last run of [`holds`] at `at_ms`, when that run is still open.
pub fn holding(votes: &[Vote], live: &[String], open_ms: u64, at_ms: u64) -> Option<Hold> {
    let last = *holds(votes, live, open_ms, at_ms).last()?;
    last.to_ms.is_none().then_some(last)
}

fn decision_for(hold: Hold, close_ms: u64) -> Option<Decision> {
    let end = hold.to_ms.unwrap_or(close_ms);
    let held_to_ms = hold.from_ms.checked_add(HOLD_MS)?;
    (held_to_ms <= end).then_some(Decision::Decided {
        colour: hold.colour,
        held_from_ms: hold.from_ms,
        held_to_ms,
    })
}

/// Distinct authors of votes with `open_ms <= at_ms <= close_ms`, in author order.
///
/// An empty window, including `open_ms > close_ms`, has no live voters.
pub fn live_voters(votes: &[Vote], open_ms: u64, close_ms: u64) -> Vec<String> {
    let mut authors = BTreeSet::new();
    for vote in votes {
        if open_ms <= vote.at_ms && vote.at_ms <= close_ms {
            authors.insert(vote.author.clone());
        }
    }
    authors.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Colour::{Blue, Red};

    fn live(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn vote(author: &str, colour: Colour, at_ms: u64) -> Vote {
        Vote {
            author: author.to_owned(),
            colour,
            at_ms,
        }
    }

    fn blue_at(from: u64) -> Decision {
        Decision::Decided {
            colour: Blue,
            held_from_ms: from,
            held_to_ms: from + HOLD_MS,
        }
    }

    #[test]
    fn unanimous_at_open_decides_after_one_hour() {
        let open = 123;
        let votes = [
            vote("a", Blue, open),
            vote("b", Blue, open),
            vote("c", Blue, open),
        ];
        assert_eq!(
            decide(&votes, &live(&["a", "b", "c"]), open, open + HOLD_MS),
            blue_at(open)
        );
    }

    #[test]
    fn interrupted_majority_restarts_the_hold() {
        let t = 100;
        let regained = t + HOLD_MS * 2 / 3;
        let votes = [
            vote("a", Blue, t),
            vote("b", Blue, t),
            vote("b", Red, t + HOLD_MS / 2),
            vote("b", Blue, regained),
        ];
        assert_eq!(
            decide(&votes, &live(&["a", "b", "c"]), 0, regained + HOLD_MS),
            blue_at(regained)
        );
    }

    #[test]
    fn late_majority_cannot_finish_before_close() {
        let votes = [vote("a", Blue, HOLD_MS / 2), vote("b", Blue, HOLD_MS / 2)];
        assert_eq!(
            decide(&votes, &live(&["a", "b", "c"]), 0, HOLD_MS),
            Decision::Undecided
        );
    }

    #[test]
    fn exactly_half_never_decides() {
        let votes = [
            vote("a", Blue, 0),
            vote("b", Blue, 0),
            vote("c", Red, 0),
            vote("d", Red, 0),
        ];
        assert_eq!(
            decide(&votes, &live(&["a", "b", "c", "d"]), 0, HOLD_MS * 2),
            Decision::Undecided
        );
    }

    #[test]
    fn only_live_in_window_votes_count_and_new_votes_replace_old_ones() {
        let votes = [
            vote("a", Red, 0),
            vote("a", Blue, 1),
            vote("outsider", Red, 1),
            vote("b", Blue, 1),
            vote("b", Blue, 2),
            vote("a", Red, HOLD_MS + 2),
        ];
        assert_eq!(
            decide(&votes, &live(&["a", "b", "c"]), 1, HOLD_MS + 1),
            blue_at(1)
        );
        assert_eq!(
            decide(&[vote("outsider", Blue, 0)], &live(&["a"]), 0, HOLD_MS),
            Decision::Undecided
        );
        let replaced = [vote("a", Blue, 0), vote("a", Red, 1)];
        assert_eq!(
            decide(&replaced, &live(&["a", "b", "c"]), 0, HOLD_MS * 2),
            Decision::Undecided
        );
    }

    #[test]
    fn input_order_and_simultaneous_conflicts_are_deterministic() {
        let votes = [vote("a", Blue, 0), vote("a", Red, 0), vote("b", Blue, 0)];
        let mut reversed = votes.clone();
        reversed.reverse();
        let voters = live(&["a", "b", "c"]);
        assert_eq!(decide(&votes, &voters, 0, HOLD_MS), blue_at(0));
        assert_eq!(
            decide(&votes, &voters, 0, HOLD_MS),
            decide(&reversed, &voters, 0, HOLD_MS)
        );
    }

    #[test]
    fn records_before_open_or_after_close_cannot_create_a_decision() {
        let voters = live(&["a", "b", "c"]);
        let before = [vote("a", Blue, 0), vote("b", Blue, 0)];
        assert_eq!(
            decide(&before, &voters, 1, HOLD_MS + 1),
            Decision::Undecided
        );
        let after = [
            vote("a", Blue, HOLD_MS / 2),
            vote("b", Blue, HOLD_MS / 2),
            vote("a", Red, HOLD_MS * 2),
        ];
        assert_eq!(decide(&after, &voters, 0, HOLD_MS), Decision::Undecided);
        let repeated = [vote("a", Blue, 0), vote("a", Blue, 1)];
        assert_eq!(
            decide(&repeated, &voters, 0, HOLD_MS + 1),
            Decision::Undecided
        );
    }

    #[test]
    fn repeat_vote_preserves_hold_and_later_majority_does_not_replace_decision() {
        let votes = [
            vote("a", Blue, 0),
            vote("a", Blue, HOLD_MS / 2),
            vote("a", Red, HOLD_MS),
        ];
        assert_eq!(decide(&votes, &live(&["a"]), 0, HOLD_MS * 3), blue_at(0));
    }

    #[test]
    fn empty_and_reversed_windows_are_undecided() {
        let votes = [vote("a", Blue, 0)];
        assert_eq!(decide(&[], &live(&["a"]), 0, HOLD_MS), Decision::Undecided);
        assert_eq!(decide(&votes, &[], 0, HOLD_MS), Decision::Undecided);
        assert_eq!(
            decide(&votes, &live(&["a"]), HOLD_MS, 0),
            Decision::Undecided
        );
    }

    #[test]
    fn live_voters_drop_outsiders_and_repeats_and_two_of_three_decide_blue() {
        let open = 1_000;
        let votes = [
            vote("b", Blue, open),
            vote("a", Blue, open),
            vote("a", Blue, open + 1),
            vote("c", Red, open),
            vote("z", Blue, open - 1),
            vote("y", Blue, open + HOLD_MS + 1),
        ];
        let voters = live_voters(&votes, open, open + HOLD_MS);
        assert_eq!(voters, live(&["a", "b", "c"]));
        assert_eq!(decide(&votes, &voters, open, open + HOLD_MS), blue_at(open));
    }

    #[test]
    fn live_voters_keep_both_edges_and_drop_an_empty_window() {
        let votes = [
            vote("a", Blue, 5),
            vote("a", Red, 5),
            vote("b", Blue, 4),
            vote("c", Blue, 6),
        ];
        assert_eq!(live_voters(&votes, 5, 5), live(&["a"]));
        assert!(live_voters(&votes, 6, 5).is_empty());
    }

    #[test]
    fn timestamp_limit_does_not_overflow_or_shorten_a_hold() {
        let from = u64::MAX - HOLD_MS;
        let voters = live(&["a"]);
        assert_eq!(
            decide(&[vote("a", Blue, from)], &voters, from, u64::MAX),
            blue_at(from)
        );
        assert_eq!(
            decide(&[vote("a", Blue, from + 1)], &voters, from, u64::MAX),
            Decision::Undecided
        );
    }

    fn first_lasting_hold(
        votes: &[Vote],
        live: &[String],
        open_ms: u64,
        close_ms: u64,
    ) -> Decision {
        holds(votes, live, open_ms, close_ms)
            .into_iter()
            .find(|hold| hold.to_ms.unwrap_or(close_ms) - hold.from_ms >= HOLD_MS)
            .map(|hold| Decision::Decided {
                colour: hold.colour,
                held_from_ms: hold.from_ms,
                held_to_ms: hold.from_ms + HOLD_MS,
            })
            .unwrap_or(Decision::Undecided)
    }

    #[test]
    fn decide_matches_the_first_hold_lasting_an_hour() {
        let open = 123;
        let t = 100;
        let regained = t + HOLD_MS * 2 / 3;
        let from = u64::MAX - HOLD_MS;
        let window = 1_000;
        let archive = vec![
            vote("b", Blue, window),
            vote("a", Blue, window),
            vote("a", Blue, window + 1),
            vote("c", Red, window),
            vote("z", Blue, window - 1),
            vote("y", Blue, window + HOLD_MS + 1),
        ];
        let ordered = vec![vote("a", Blue, 0), vote("a", Red, 0), vote("b", Blue, 0)];
        let mut reversed = ordered.clone();
        reversed.reverse();
        let cases = [
            (
                "unanimous at open",
                vec![
                    vote("a", Blue, open),
                    vote("b", Blue, open),
                    vote("c", Blue, open),
                ],
                live(&["a", "b", "c"]),
                open,
                open + HOLD_MS,
            ),
            (
                "interrupted majority",
                vec![
                    vote("a", Blue, t),
                    vote("b", Blue, t),
                    vote("b", Red, t + HOLD_MS / 2),
                    vote("b", Blue, regained),
                ],
                live(&["a", "b", "c"]),
                0,
                regained + HOLD_MS,
            ),
            (
                "late majority",
                vec![vote("a", Blue, HOLD_MS / 2), vote("b", Blue, HOLD_MS / 2)],
                live(&["a", "b", "c"]),
                0,
                HOLD_MS,
            ),
            (
                "exactly half",
                vec![
                    vote("a", Blue, 0),
                    vote("b", Blue, 0),
                    vote("c", Red, 0),
                    vote("d", Red, 0),
                ],
                live(&["a", "b", "c", "d"]),
                0,
                HOLD_MS * 2,
            ),
            (
                "only live votes inside the window",
                vec![
                    vote("a", Red, 0),
                    vote("a", Blue, 1),
                    vote("outsider", Red, 1),
                    vote("b", Blue, 1),
                    vote("b", Blue, 2),
                    vote("a", Red, HOLD_MS + 2),
                ],
                live(&["a", "b", "c"]),
                1,
                HOLD_MS + 1,
            ),
            (
                "outsider alone",
                vec![vote("outsider", Blue, 0)],
                live(&["a"]),
                0,
                HOLD_MS,
            ),
            (
                "replaced vote",
                vec![vote("a", Blue, 0), vote("a", Red, 1)],
                live(&["a", "b", "c"]),
                0,
                HOLD_MS * 2,
            ),
            ("input order", ordered, live(&["a", "b", "c"]), 0, HOLD_MS),
            (
                "reversed input order",
                reversed,
                live(&["a", "b", "c"]),
                0,
                HOLD_MS,
            ),
            (
                "votes before open",
                vec![vote("a", Blue, 0), vote("b", Blue, 0)],
                live(&["a", "b", "c"]),
                1,
                HOLD_MS + 1,
            ),
            (
                "vote after close",
                vec![
                    vote("a", Blue, HOLD_MS / 2),
                    vote("b", Blue, HOLD_MS / 2),
                    vote("a", Red, HOLD_MS * 2),
                ],
                live(&["a", "b", "c"]),
                0,
                HOLD_MS,
            ),
            (
                "repeat by the only voter",
                vec![vote("a", Blue, 0), vote("a", Blue, 1)],
                live(&["a", "b", "c"]),
                0,
                HOLD_MS + 1,
            ),
            (
                "repeat preserves the hold",
                vec![
                    vote("a", Blue, 0),
                    vote("a", Blue, HOLD_MS / 2),
                    vote("a", Red, HOLD_MS),
                ],
                live(&["a"]),
                0,
                HOLD_MS * 3,
            ),
            ("no votes", Vec::new(), live(&["a"]), 0, HOLD_MS),
            (
                "no live voters",
                vec![vote("a", Blue, 0)],
                Vec::new(),
                0,
                HOLD_MS,
            ),
            (
                "reversed window",
                vec![vote("a", Blue, 0)],
                live(&["a"]),
                HOLD_MS,
                0,
            ),
            (
                "two of three live voters",
                archive.clone(),
                live_voters(&archive, window, window + HOLD_MS),
                window,
                window + HOLD_MS,
            ),
            (
                "hold reaches the timestamp limit",
                vec![vote("a", Blue, from)],
                live(&["a"]),
                from,
                u64::MAX,
            ),
            (
                "hold would overflow the timestamp",
                vec![vote("a", Blue, from + 1)],
                live(&["a"]),
                from,
                u64::MAX,
            ),
        ];
        for (name, votes, voters, open_ms, close_ms) in cases {
            assert_eq!(
                decide(&votes, &voters, open_ms, close_ms),
                first_lasting_hold(&votes, &voters, open_ms, close_ms),
                "{name}"
            );
        }
    }

    #[test]
    fn holds_keep_each_majority_run_and_holding_is_the_open_run() {
        let t = 1_000_000;
        let twenty = 20 * 60 * 1000;
        let forty = 40 * 60 * 1000;
        let fifty = 50 * 60 * 1000;
        let voters = live(&["a", "b", "c"]);
        let votes = [
            vote("a", Blue, t),
            vote("b", Blue, t),
            vote("b", Red, t + twenty),
            vote("b", Blue, t + forty),
        ];
        assert_eq!(
            holds(&votes, &voters, t, t + fifty),
            vec![
                Hold {
                    colour: Blue,
                    from_ms: t,
                    to_ms: Some(t + twenty),
                },
                Hold {
                    colour: Blue,
                    from_ms: t + forty,
                    to_ms: None,
                },
            ]
        );
        assert_eq!(
            holding(&votes, &voters, t, t + fifty),
            Some(Hold {
                colour: Blue,
                from_ms: t + forty,
                to_ms: None,
            })
        );
        assert_eq!(
            holding(&votes, &voters, t, t + twenty + 10 * 60 * 1000),
            None
        );
    }
}
