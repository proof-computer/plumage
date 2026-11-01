//! The day's closing rule over replicated votes and their hourly expiry.

use crate::colour::Colour;
use std::collections::{BTreeMap, BTreeSet};

pub const HOLD_MS: u64 = 3_600_000;
pub const RULE: &str = "The day's colour is the first colour held by more than half of the live voters for a continuous hour ending at or before close; otherwise the day is undecided.";
pub const LIVE_RULE: &str = "The live voters at a time are the authors whose newest vote at or before that time, cast between the day's open (00:00 UTC) and close (23:00 UTC), is less than one hour old. An author leaves the live voters one hour after its last vote and returns with its next vote.";
pub const HOLDING_RULE: &str = "A colour holds while more than half of the live voters' current votes name it. A hold starts at the moment that makes it more than half and ends at the moment that makes it half or fewer; either moment is a vote or an expiry, the instant an author's newest vote becomes one hour old. No colour holds while there are no live voters.";

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
/// `to_ms` is the vote or expiry that ended the run. It is absent while the run
/// is still held after the last replayed event.
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
pub fn decide(votes: &[Vote], open_ms: u64, close_ms: u64) -> Decision {
    holds(votes, open_ms, close_ms)
        .into_iter()
        .find_map(|hold| decision_for(hold, close_ms))
        .unwrap_or(Decision::Undecided)
}

/// Every majority run, in replay order. The filter, sort, majority test and
/// first-colour choice are the ones [`decide`] uses.
pub fn holds(votes: &[Vote], open_ms: u64, until_ms: u64) -> Vec<Hold> {
    let votes: Vec<_> = votes
        .iter()
        .filter(|v| open_ms <= v.at_ms && v.at_ms <= until_ms)
        .cloned()
        .collect();
    let mut instants = BTreeSet::new();
    for vote in &votes {
        instants.insert(vote.at_ms);
        if let Some(expiry) = vote.at_ms.checked_add(HOLD_MS).filter(|t| *t <= until_ms) {
            instants.insert(expiry);
        }
    }

    let mut runs: Vec<Hold> = Vec::new();
    for at_ms in instants {
        let live = live_voters(&votes, open_ms, at_ms);
        let counts = crate::tally::counts(&votes, open_ms, at_ms);
        let majority = counts
            .iter()
            .find_map(|(&colour, &count)| (count > live.len() / 2).then_some(colour));
        let held_colour = runs
            .last()
            .filter(|hold| hold.to_ms.is_none())
            .map(|hold| hold.colour);
        if majority != held_colour {
            if let Some(open) = runs.last_mut().filter(|hold| hold.to_ms.is_none()) {
                open.to_ms = Some(at_ms);
            }
            if let Some(colour) = majority {
                runs.push(Hold {
                    colour,
                    from_ms: at_ms,
                    to_ms: None,
                });
            }
        }
    }
    runs
}

/// The last run of [`holds`] at `at_ms`, when that run is still open.
pub fn holding(votes: &[Vote], open_ms: u64, at_ms: u64) -> Option<Hold> {
    let last = *holds(votes, open_ms, at_ms).last()?;
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

/// Authors whose newest vote in `open_ms..=at_ms` is less than [`HOLD_MS`]
/// old, in author order. An empty or reversed window has no live voters.
pub fn live_voters(votes: &[Vote], open_ms: u64, at_ms: u64) -> Vec<String> {
    let mut newest: BTreeMap<&str, &Vote> = BTreeMap::new();
    for vote in votes
        .iter()
        .filter(|v| open_ms <= v.at_ms && v.at_ms <= at_ms)
    {
        newest
            .entry(&vote.author)
            .and_modify(|have| {
                if (vote.at_ms, vote.colour) > (have.at_ms, have.colour) {
                    *have = vote;
                }
            })
            .or_insert(vote);
    }
    newest
        .into_iter()
        .filter(|(_, vote)| at_ms - vote.at_ms < HOLD_MS)
        .map(|(author, _)| author.to_owned())
        .collect()
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
        assert_eq!(decide(&votes, open, open + HOLD_MS), blue_at(open));
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
        assert_eq!(decide(&votes, 0, regained + HOLD_MS), blue_at(regained));
    }

    #[test]
    fn late_majority_cannot_finish_before_close() {
        let votes = [vote("a", Blue, HOLD_MS / 2), vote("b", Blue, HOLD_MS / 2)];
        assert_eq!(decide(&votes, 0, HOLD_MS), Decision::Undecided);
    }

    #[test]
    fn exactly_half_never_decides() {
        let votes = [
            vote("a", Blue, 0),
            vote("b", Blue, 0),
            vote("c", Red, 0),
            vote("d", Red, 0),
        ];
        assert_eq!(decide(&votes, 0, HOLD_MS * 2), Decision::Undecided);
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
        assert_eq!(decide(&votes, 1, HOLD_MS + 1), blue_at(1));
        assert_eq!(decide(&[vote("outsider", Blue, 0)], 0, HOLD_MS), blue_at(0));
        let replaced = [vote("a", Blue, 0), vote("a", Red, 1)];
        assert_eq!(
            decide(&replaced, 0, HOLD_MS * 2),
            Decision::Decided {
                colour: Red,
                held_from_ms: 1,
                held_to_ms: 1 + HOLD_MS
            }
        );
    }

    #[test]
    fn input_order_and_simultaneous_conflicts_are_deterministic() {
        let votes = [vote("a", Blue, 0), vote("a", Red, 0), vote("b", Blue, 0)];
        let mut reversed = votes.clone();
        reversed.reverse();
        assert_eq!(decide(&votes, 0, HOLD_MS), blue_at(0));
        assert_eq!(decide(&votes, 0, HOLD_MS), decide(&reversed, 0, HOLD_MS));
    }

    #[test]
    fn records_before_open_or_after_close_cannot_create_a_decision() {
        let before = [vote("a", Blue, 0), vote("b", Blue, 0)];
        assert_eq!(decide(&before, 1, HOLD_MS + 1), Decision::Undecided);
        let after = [
            vote("a", Blue, HOLD_MS / 2),
            vote("b", Blue, HOLD_MS / 2),
            vote("a", Red, HOLD_MS * 2),
        ];
        assert_eq!(decide(&after, 0, HOLD_MS), Decision::Undecided);
        let repeated = [vote("a", Blue, 0), vote("a", Blue, 1)];
        assert_eq!(decide(&repeated, 0, HOLD_MS + 1), blue_at(0));
    }

    #[test]
    fn repeat_vote_preserves_hold_and_later_majority_does_not_replace_decision() {
        let votes = [
            vote("a", Blue, 0),
            vote("a", Blue, HOLD_MS / 2),
            vote("a", Red, HOLD_MS),
        ];
        assert_eq!(decide(&votes, 0, HOLD_MS * 3), blue_at(0));
    }

    #[test]
    fn empty_and_reversed_windows_are_undecided() {
        let votes = [vote("a", Blue, 0)];
        assert_eq!(decide(&[], 0, HOLD_MS), Decision::Undecided);
        assert_eq!(decide(&votes, HOLD_MS, 0), Decision::Undecided);
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
        let voters = live_voters(&votes, open, open + 1);
        assert_eq!(voters, live(&["a", "b", "c"]));
        assert_eq!(decide(&votes, open, open + HOLD_MS), blue_at(open));
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
        assert_eq!(
            decide(&[vote("a", Blue, from)], from, u64::MAX),
            blue_at(from)
        );
        assert_eq!(
            decide(&[vote("a", Blue, from + 1)], from, u64::MAX),
            Decision::Undecided
        );
    }

    fn first_lasting_hold(votes: &[Vote], open_ms: u64, close_ms: u64) -> Decision {
        holds(votes, open_ms, close_ms)
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
                0,
                regained + HOLD_MS,
            ),
            (
                "late majority",
                vec![vote("a", Blue, HOLD_MS / 2), vote("b", Blue, HOLD_MS / 2)],
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
                1,
                HOLD_MS + 1,
            ),
            (
                "outsider alone",
                vec![vote("outsider", Blue, 0)],
                0,
                HOLD_MS,
            ),
            (
                "replaced vote",
                vec![vote("a", Blue, 0), vote("a", Red, 1)],
                0,
                HOLD_MS * 2,
            ),
            ("input order", ordered, 0, HOLD_MS),
            ("reversed input order", reversed, 0, HOLD_MS),
            (
                "votes before open",
                vec![vote("a", Blue, 0), vote("b", Blue, 0)],
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
                0,
                HOLD_MS,
            ),
            (
                "repeat by the only voter",
                vec![vote("a", Blue, 0), vote("a", Blue, 1)],
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
                0,
                HOLD_MS * 3,
            ),
            ("no votes", Vec::new(), 0, HOLD_MS),
            ("solo voter", vec![vote("a", Blue, 0)], 0, HOLD_MS),
            ("reversed window", vec![vote("a", Blue, 0)], HOLD_MS, 0),
            (
                "two of three live voters",
                archive.clone(),
                window,
                window + HOLD_MS,
            ),
            (
                "hold reaches the timestamp limit",
                vec![vote("a", Blue, from)],
                from,
                u64::MAX,
            ),
            (
                "hold would overflow the timestamp",
                vec![vote("a", Blue, from + 1)],
                from,
                u64::MAX,
            ),
        ];
        for (name, votes, open_ms, close_ms) in cases {
            assert_eq!(
                decide(&votes, open_ms, close_ms),
                first_lasting_hold(&votes, open_ms, close_ms),
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
        let votes = [
            vote("a", Blue, t),
            vote("b", Blue, t),
            vote("b", Red, t + twenty),
            vote("b", Blue, t + forty),
        ];
        assert_eq!(
            holds(&votes, t, t + fifty),
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
            holding(&votes, t, t + fifty),
            Some(Hold {
                colour: Blue,
                from_ms: t + forty,
                to_ms: None,
            })
        );
        assert_eq!(holding(&votes, t, t + twenty + 10 * 60 * 1000), None);
    }
    fn at(hour: u64, minute: u64) -> u64 {
        hour * HOLD_MS + minute * 60_000
    }

    fn run(colour: Colour, from_ms: u64, to_ms: Option<u64>) -> Hold {
        Hold {
            colour,
            from_ms,
            to_ms,
        }
    }

    #[test]
    fn live_voters_and_counts_expire_at_exactly_one_hour() {
        let votes = [vote("a", Red, at(10, 0)), vote("b", Blue, at(10, 30))];
        for (time, authors) in [
            (at(10, 59), live(&["a", "b"])),
            (at(11, 0), live(&["b"])),
            (at(11, 1), live(&["b"])),
            (at(11, 30), live(&[])),
        ] {
            assert_eq!(live_voters(&votes, 0, time), authors);
        }
        assert_eq!(
            crate::tally::counts(&votes, 0, at(10, 59)),
            BTreeMap::from([(Blue, 1), (Red, 1)])
        );
        assert_eq!(
            crate::tally::counts(&votes, 0, at(11, 1)),
            BTreeMap::from([(Blue, 1)])
        );
        let mut returned = votes.to_vec();
        returned.push(vote("a", Red, at(11, 2)));
        assert_eq!(live_voters(&returned, 0, at(11, 2)), live(&["a", "b"]));
    }

    fn expiry_keeps_fixture() -> Vec<Vote> {
        vec![
            vote("e", Red, at(9, 30)),
            vote("f", Red, at(9, 30)),
            vote("a", Blue, at(10, 0)),
            vote("b", Blue, at(10, 0)),
            vote("c", Blue, at(10, 0)),
            vote("d", Blue, at(10, 0)),
        ]
    }

    #[test]
    fn expiry_keeps_a_hold_when_the_majority_colour_stays_the_same() {
        let votes = expiry_keeps_fixture();
        assert_eq!(
            holds(&votes, 0, at(10, 50)),
            vec![
                run(Red, at(9, 30), Some(at(10, 0))),
                run(Blue, at(10, 0), None)
            ]
        );
        assert_eq!(
            live_voters(&votes, 0, at(10, 30)),
            live(&["a", "b", "c", "d"])
        );
        assert_eq!(
            crate::tally::counts(&votes, 0, at(10, 30)),
            BTreeMap::from([(Blue, 4)])
        );
        assert_eq!(decide(&votes, 0, at(11, 0)), blue_at(at(10, 0)));
    }

    #[test]
    fn expiry_can_end_or_start_a_hold_without_a_vote() {
        let votes = [
            vote("a", Blue, at(9, 20)),
            vote("c", Red, at(9, 50)),
            vote("b", Blue, at(10, 0)),
        ];
        assert_eq!(
            holds(&votes, 0, at(10, 50)),
            vec![
                run(Blue, at(9, 20), Some(at(9, 50))),
                run(Blue, at(10, 0), Some(at(10, 20))),
                run(Blue, at(10, 50), None)
            ]
        );
        assert_eq!(
            holding(&votes, 0, at(10, 19)),
            Some(run(Blue, at(10, 0), None))
        );
        assert_eq!(holding(&votes, 0, at(10, 20)), None);
        let votes = [
            vote("a", Blue, at(9, 20)),
            vote("b", Blue, at(9, 20)),
            vote("c", Blue, at(10, 0)),
            vote("d", Red, at(10, 0)),
            vote("e", Red, at(10, 0)),
        ];
        assert_eq!(
            holds(&votes, 0, at(10, 50)),
            vec![
                run(Blue, at(9, 20), Some(at(10, 20))),
                run(Red, at(10, 20), None)
            ]
        );
        assert_eq!(decide(&votes, 0, at(23, 0)), blue_at(at(9, 20)));
    }

    fn first_day_fixture() -> Vec<Vote> {
        use Colour::Black;
        [
            ("corncrake", Blue, 13, 38),
            ("corncrake", Black, 13, 42),
            ("corncrake", Black, 14, 20),
            ("corncrake", Blue, 14, 51),
            ("corncrake", Blue, 15, 28),
            ("avocet", Blue, 14, 37),
            ("avocet", Blue, 15, 30),
            ("avocet", Blue, 16, 28),
            ("whimbrel", Blue, 15, 40),
            ("whimbrel", Blue, 16, 36),
            ("whimbrel", Blue, 17, 28),
            ("nightingale", Black, 16, 40),
            ("nightingale", Black, 17, 20),
            ("nightingale", Blue, 17, 50),
            ("nightingale", Blue, 18, 28),
            ("heron", Black, 18, 34),
            ("heron", Black, 18, 40),
            ("heron", Blue, 18, 47),
            ("heron", Blue, 19, 38),
            ("shrike", Blue, 19, 38),
        ]
        .into_iter()
        .map(|(author, colour, hour, minute)| vote(author, colour, at(hour, minute)))
        .collect()
    }

    #[test]
    fn first_day_decides_with_only_the_voters_live_at_each_instant() {
        let votes = first_day_fixture();
        assert_eq!(decide(&votes, 0, at(23, 0)), blue_at(at(14, 51)));
        assert_eq!(
            holds(&votes, 0, at(23, 0)),
            vec![
                run(Blue, at(13, 38), Some(at(13, 42))),
                run(Colour::Black, at(13, 42), Some(at(14, 37))),
                run(Blue, at(14, 51), Some(at(17, 28))),
                run(Blue, at(17, 50), Some(at(18, 34))),
                run(Blue, at(18, 47), Some(at(20, 38))),
            ]
        );
        assert_eq!(
            live_voters(&votes, 0, at(15, 51)),
            live(&["avocet", "corncrake", "whimbrel"])
        );
        assert_eq!(
            live_voters(&votes, 0, at(17, 50)),
            live(&["nightingale", "whimbrel"])
        );
        assert_eq!(
            live_voters(&votes, 0, at(19, 38)),
            live(&["heron", "shrike"])
        );
    }

    #[test]
    fn vote_and_expiry_replay_is_independent_of_input_order() {
        for votes in [expiry_keeps_fixture(), first_day_fixture()] {
            let expected = (decide(&votes, 0, at(23, 0)), holds(&votes, 0, at(23, 0)));
            let mut reversed = votes.clone();
            reversed.reverse();
            assert_eq!(
                (
                    decide(&reversed, 0, at(23, 0)),
                    holds(&reversed, 0, at(23, 0))
                ),
                expected
            );
            for shift in [1, 3, 7] {
                let mut rotated = votes.clone();
                let len = rotated.len();
                rotated.rotate_left(shift % len);
                assert_eq!(
                    (
                        decide(&rotated, 0, at(23, 0)),
                        holds(&rotated, 0, at(23, 0))
                    ),
                    expected
                );
            }
        }
    }
}
