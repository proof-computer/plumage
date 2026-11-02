//! Colour-domain action grammar and validation; fetching is only an intent.

use crate::{
    colour::Colour,
    evidence::{Allowlist, Evidence, EvidenceId},
    tally,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

// Q-20261023-7xbm: three fetches per activation, seeded by peer-id SHA-256.
pub const FETCHES_PER_ACTIVATION: usize = 3;
pub const ARGUMENT_CHARS: usize = 400;
pub const MAX_CITATIONS: usize = 8;
pub const STANCE_PREFER_MIN: usize = 1;
pub const STANCE_PREFER_MAX: usize = 3;
pub const STANCE_ACCEPT_MAX: usize = 4;
pub const STANCE_REFUSE_MAX: usize = 3;
/// An argument id: `r:a:` and 64 lowercase hex digits.
const REPLY_TO_PATTERN: &str = "^r:a:[0-9a-f]{64}$";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Fetch {
        url: Url,
    },
    Argue {
        text: String,
        cites: Vec<EvidenceId>,
        /// One other author's argument in this run. Absent means the argument
        /// answers nothing, and the field is omitted from its JSON.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },
    Stance {
        prefer: Vec<Colour>,
        accept: Vec<Colour>,
        refuse: Vec<Colour>,
        cites: Vec<EvidenceId>,
    },
    Vote {
        colour: Colour,
    },
}

/// The known evidence records identify which ids this author fetched in this
/// run, and distinguish another author's ids from unknown ids. The caller
/// counts every completed fetch, whether cited or not, in this activation.
pub struct Context<'a> {
    pub author: &'a str,
    pub run: &'a str,
    pub evidence: &'a [Evidence],
    pub fetches_this_activation: usize,
    /// Argument ids in this run, each with the author who wrote it.
    pub arguments: &'a [(String, String)],
    /// Vote and evidence record ids in this run.
    pub other_records: &'a [String],
    pub stance: Option<&'a tally::Stance>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    HostNotAllowed,
    BudgetExhausted,
    UnknownCitation,
    ForeignCitation,
    TextTooLong,
    TooManyCitations,
    UnknownReply,
    ReplyToNonArgument,
    SelfReply,
    VoteBeforeReading,
    IdInText,
    StanceBounds,
    StanceColourRepeated,
    StanceRefusalUncited,
    VoteBeforeStance,
    VoteOutsideStance,
}

impl Refusal {
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::VoteBeforeReading => {
                Some("fetch one source and post one argument first; a vote needs both in this run")
            }
            Self::IdInText => {
                Some("put evidence ids in cites and argument ids in reply_to, not in text")
            }
            Self::StanceBounds => Some("prefer needs 1 to 3 colours, accept 0 to 4, refuse 0 to 3"),
            Self::StanceColourRepeated => {
                Some("name each colour at most once across prefer, accept and refuse")
            }
            Self::StanceRefusalUncited => Some(
                "a refusal cites at least one evidence id you fetched this run; cite one or leave refuse empty",
            ),
            Self::VoteBeforeStance => Some(
                "declare a stance first; a vote needs one fetch, a stance and one argument in this run",
            ),
            Self::VoteOutsideStance => Some(
                "vote a colour in your current prefer or accept lists, or declare a new stance first",
            ),
            _ => None,
        }
    }
}

pub fn validate(action: &Action, context: &Context<'_>) -> Result<(), Refusal> {
    match action {
        Action::Fetch { url } => {
            if !Allowlist::permits(url) {
                return Err(Refusal::HostNotAllowed);
            }
            if context.fetches_this_activation >= FETCHES_PER_ACTIVATION {
                return Err(Refusal::BudgetExhausted);
            }
        }
        Action::Argue {
            text,
            cites,
            reply_to,
        } => {
            if text.chars().count() > ARGUMENT_CHARS {
                return Err(Refusal::TextTooLong);
            }
            if cites.len() > MAX_CITATIONS {
                return Err(Refusal::TooManyCitations);
            }
            let mut hex_run = 0;
            if text.contains("r:a:")
                || text.chars().any(|character| {
                    hex_run = if character.is_ascii_hexdigit() {
                        hex_run + 1
                    } else {
                        0
                    };
                    hex_run >= 64
                })
            {
                return Err(Refusal::IdInText);
            }
            for id in cites {
                let evidence = context
                    .evidence
                    .iter()
                    .find(|e| e.id() == id && e.run() == context.run)
                    .ok_or(Refusal::UnknownCitation)?;
                if evidence.author() != context.author {
                    return Err(Refusal::ForeignCitation);
                }
            }
            if let Some(target) = reply_to {
                let known_argument = context.arguments.iter().find(|(id, _)| id == target);
                let known_other = context.other_records.iter().any(|id| id == target);
                if known_argument.is_none() && !known_other {
                    return Err(Refusal::UnknownReply);
                }
                if known_argument.is_none() {
                    return Err(Refusal::ReplyToNonArgument);
                }
                if known_argument.is_some_and(|(_, author)| author == context.author) {
                    return Err(Refusal::SelfReply);
                }
            }
        }
        Action::Stance {
            prefer,
            accept,
            refuse,
            cites,
        } => {
            if !(STANCE_PREFER_MIN..=STANCE_PREFER_MAX).contains(&prefer.len())
                || accept.len() > STANCE_ACCEPT_MAX
                || refuse.len() > STANCE_REFUSE_MAX
            {
                return Err(Refusal::StanceBounds);
            }
            let mut seen = std::collections::BTreeSet::new();
            if prefer
                .iter()
                .chain(accept)
                .chain(refuse)
                .any(|colour| !seen.insert(colour))
            {
                return Err(Refusal::StanceColourRepeated);
            }
            if cites.len() > MAX_CITATIONS {
                return Err(Refusal::TooManyCitations);
            }
            if !refuse.is_empty() && cites.is_empty() {
                return Err(Refusal::StanceRefusalUncited);
            }
            for id in cites {
                let evidence = context
                    .evidence
                    .iter()
                    .find(|e| e.id() == id && e.run() == context.run)
                    .ok_or(Refusal::UnknownCitation)?;
                if evidence.author() != context.author {
                    return Err(Refusal::ForeignCitation);
                }
            }
        }
        Action::Vote { colour } => {
            let fetched = context.evidence.iter().any(|evidence| {
                evidence.author() == context.author && evidence.run() == context.run
            });
            let argued = context
                .arguments
                .iter()
                .any(|(_, author)| author == context.author);
            if !fetched || !argued {
                return Err(Refusal::VoteBeforeReading);
            }
            let stance = context.stance.ok_or(Refusal::VoteBeforeStance)?;
            if !stance.prefer.contains(colour) && !stance.accept.contains(colour) {
                return Err(Refusal::VoteOutsideStance);
            }
        }
    }
    Ok(())
}

/// JSON schema consumed by the later runtime's generation grammar.
pub fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "oneOf": [
            {"type": "object", "additionalProperties": false,
             "required": ["action", "url"],
             "properties": {"action": {"const": "fetch"}, "url": {"type": "string"}}},
            {"type": "object", "additionalProperties": false,
             "required": ["action", "text", "cites"],
             "properties": {"action": {"const": "argue"},
                 "text": {"type": "string", "maxLength": ARGUMENT_CHARS},
                 "cites": {"type": "array", "maxItems": MAX_CITATIONS,
                           "items": {"type": "string"}},
                 "reply_to": {"type": "string", "pattern": REPLY_TO_PATTERN}}},
            {"type": "object", "additionalProperties": false,
             "required": ["action", "prefer", "accept", "refuse", "cites"],
             "properties": {"action": {"const": "stance"},
                 "prefer": {"type": "array", "minItems": STANCE_PREFER_MIN, "maxItems": STANCE_PREFER_MAX,
                            "items": {"type": "string", "enum": Colour::ALL.map(|c| c.name())}},
                 "accept": {"type": "array", "maxItems": STANCE_ACCEPT_MAX,
                            "items": {"type": "string", "enum": Colour::ALL.map(|c| c.name())}},
                 "refuse": {"type": "array", "maxItems": STANCE_REFUSE_MAX,
                            "items": {"type": "string", "enum": Colour::ALL.map(|c| c.name())}},
                 "cites": {"type": "array", "maxItems": MAX_CITATIONS, "items": {"type": "string"}}}},
            {"type": "object", "additionalProperties": false,
             "required": ["action", "colour"],
             "properties": {"action": {"const": "vote"},
                 "colour": {"type": "string", "enum": Colour::ALL.map(|c| c.name())}}}
        ]
    })
}

/// Reduce the entire big-endian SHA-256 integer, including all 32 bytes.
pub fn seed_offset(peer_id: &str, choices: usize) -> usize {
    if choices == 0 {
        return 0;
    }
    Sha256::digest(peer_id.as_bytes())
        .iter()
        .fold(0u128, |offset, byte| {
            (offset * 256 + u128::from(*byte)) % choices as u128
        }) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKI: &str = "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/events/10/23";

    fn context(evidence: &[Evidence], fetches: usize) -> Context<'_> {
        Context {
            author: "a",
            run: "today",
            evidence,
            fetches_this_activation: fetches,
            arguments: &[],
            other_records: &[],
            stance: None,
        }
    }

    fn argue(text: &str, cites: Vec<EvidenceId>) -> Action {
        Action::Argue {
            text: text.into(),
            cites,
            reply_to: None,
        }
    }

    fn argument_id(seed: &str) -> String {
        format!("r:a:{seed}")
    }

    #[test]
    fn fetch_requires_allowlist_and_remaining_budget() {
        let fetch = Action::Fetch {
            url: Url::parse(WIKI).unwrap(),
        };
        for count in [0, 1, 2] {
            assert_eq!(validate(&fetch, &context(&[], count)), Ok(()));
        }
        for count in [3, 4, usize::MAX] {
            assert_eq!(
                validate(&fetch, &context(&[], count)),
                Err(Refusal::BudgetExhausted)
            );
        }
        let foreign = Action::Fetch {
            url: Url::parse("https://example.com/").unwrap(),
        };
        assert_eq!(
            validate(&foreign, &context(&[], 0)),
            Err(Refusal::HostNotAllowed)
        );
        // The budget applies only to fetches.
        assert_eq!(validate(&argue("blue", vec![]), &context(&[], 3)), Ok(()));
    }

    #[test]
    fn fetch_accepts_another_peers_edition_but_refuses_unlisted_editions() {
        for (edition, expected) in [("de", Ok(())), ("it", Err(Refusal::HostNotAllowed))] {
            let action = Action::Fetch {
                url: Url::parse(&format!(
                    "https://api.wikimedia.org/feed/v1/wikipedia/{edition}/onthisday/events/10/23"
                ))
                .unwrap(),
            };
            assert_eq!(validate(&action, &context(&[], 0)), expected);
        }
    }

    #[test]
    fn vote_requires_own_evidence_in_this_run_and_own_argument() {
        let vote = Action::Vote {
            colour: Colour::Blue,
        };
        let own = [("argument".into(), "a".into())];
        let foreign = [("argument".into(), "b".into())];
        assert_eq!(
            validate(&vote, &context(&[], 0)),
            Err(Refusal::VoteBeforeReading)
        );
        for (author, run) in [("a", "today"), ("b", "today"), ("a", "yesterday")] {
            let records = [Evidence::new(
                author.into(),
                run.into(),
                Url::parse(WIKI).unwrap(),
                0,
                "fact",
            )];
            let mut ctx = context(&records, 3);
            for arguments in [&[][..], &foreign[..]] {
                ctx.arguments = arguments;
                assert_eq!(validate(&vote, &ctx), Err(Refusal::VoteBeforeReading));
            }
            ctx.arguments = &own;
            assert_eq!(
                validate(&vote, &ctx),
                if author == "a" && run == "today" {
                    Err(Refusal::VoteBeforeStance)
                } else {
                    Err(Refusal::VoteBeforeReading)
                }
            );
        }
        let mut ctx = context(&[], 0);
        ctx.arguments = &own;
        assert_eq!(validate(&vote, &ctx), Err(Refusal::VoteBeforeReading));
    }

    #[test]
    fn argument_ids_belong_in_structured_fields_and_refusals_explain_why() {
        let records = [Evidence::new(
            "a".into(),
            "today".into(),
            Url::parse(WIKI).unwrap(),
            0,
            "fact",
        )];
        let ctx = context(&records, 0);
        let cites = vec![records[0].id().clone()];
        for text in [
            "blue today".to_owned(),
            "a".repeat(63),
            format!("{}g{}", "a".repeat(63), "b".repeat(63)),
            "Ａ".repeat(64),
        ] {
            assert_eq!(validate(&argue(&text, cites.clone()), &ctx), Ok(()));
        }
        for text in [
            "a".repeat(64),
            "A".repeat(64),
            "1".repeat(65),
            "a r:a: reference".to_owned(),
        ] {
            assert_eq!(
                validate(&argue(&text, cites.clone()), &ctx),
                Err(Refusal::IdInText)
            );
        }
        assert_eq!(
            validate(&argue(&"a".repeat(ARGUMENT_CHARS + 1), cites.clone()), &ctx),
            Err(Refusal::TextTooLong)
        );
        assert_eq!(
            validate(
                &argue(&"a".repeat(64), vec![cites[0].clone(); MAX_CITATIONS + 1]),
                &ctx
            ),
            Err(Refusal::TooManyCitations)
        );
        assert_eq!(
            validate(
                &argue(&"a".repeat(64), vec![EvidenceId("unknown".into())]),
                &ctx
            ),
            Err(Refusal::IdInText)
        );
        assert_eq!(
            Refusal::IdInText.reason(),
            Some("put evidence ids in cites and argument ids in reply_to, not in text")
        );
        assert_eq!(
            Refusal::VoteBeforeReading.reason(),
            Some("fetch one source and post one argument first; a vote needs both in this run")
        );
        for refusal in [
            Refusal::HostNotAllowed,
            Refusal::BudgetExhausted,
            Refusal::UnknownCitation,
            Refusal::ForeignCitation,
            Refusal::TextTooLong,
            Refusal::TooManyCitations,
            Refusal::UnknownReply,
            Refusal::ReplyToNonArgument,
            Refusal::SelfReply,
        ] {
            assert_eq!(refusal.reason(), None);
        }
    }

    #[test]
    fn citations_require_this_author_and_this_run() {
        let records = [
            ("a", "today"),
            ("b", "today"),
            ("a", "yesterday"),
            ("b", "yesterday"),
        ]
        .map(|(a, r)| Evidence::new(a.into(), r.into(), Url::parse(WIKI).unwrap(), 0, "fact"));
        let ctx = context(&records, 0);
        assert_eq!(
            validate(&argue("blue", vec![records[0].id().clone()]), &ctx),
            Ok(())
        );
        assert_eq!(
            validate(&argue("blue", vec![records[1].id().clone()]), &ctx),
            Err(Refusal::ForeignCitation)
        );
        for id in [
            records[2].id().clone(),
            records[3].id().clone(),
            EvidenceId("unknown".into()),
        ] {
            assert_eq!(
                validate(&argue("blue", vec![id]), &ctx),
                Err(Refusal::UnknownCitation)
            );
        }
        assert_eq!(validate(&argue("", vec![]), &ctx), Ok(()));
        assert_eq!(
            validate(
                &argue(
                    "blue",
                    vec![records[0].id().clone(), records[1].id().clone()]
                ),
                &ctx
            ),
            Err(Refusal::ForeignCitation)
        );
    }

    #[test]
    fn arguments_have_unicode_character_and_citation_bounds() {
        let record = Evidence::new(
            "a".into(),
            "today".into(),
            Url::parse(WIKI).unwrap(),
            0,
            "fact",
        );
        let ids = vec![record.id().clone(); MAX_CITATIONS];
        let records = [record];
        let ctx = context(&records, 0);
        assert_eq!(
            validate(&argue(&"🦜".repeat(ARGUMENT_CHARS), ids.clone()), &ctx),
            Ok(())
        );
        assert_eq!(
            validate(&argue(&"🦜".repeat(ARGUMENT_CHARS + 1), vec![]), &ctx),
            Err(Refusal::TextTooLong)
        );
        assert_eq!(
            validate(
                &argue("blue", vec![ids[0].clone(); MAX_CITATIONS + 1]),
                &ctx
            ),
            Err(Refusal::TooManyCitations)
        );
    }

    #[test]
    fn schema_and_serde_close_the_action_and_colour_grammars() {
        let validator = jsonschema::validator_for(&schema()).unwrap();
        let mut actions = vec![
            Action::Fetch {
                url: Url::parse(WIKI).unwrap(),
            },
            argue(
                &"🦜".repeat(ARGUMENT_CHARS),
                vec![EvidenceId("id".into()); MAX_CITATIONS],
            ),
        ];
        actions.extend(Colour::ALL.map(|colour| Action::Vote { colour }));
        for action in actions {
            let value = serde_json::to_value(&action).unwrap();
            assert!(validator.is_valid(&value), "{value}");
            assert_eq!(serde_json::from_value::<Action>(value).unwrap(), action);
        }
        let declared = &schema()["oneOf"][3]["properties"]["colour"]["enum"];
        assert_eq!(declared.as_array().unwrap().len(), 24);
        for value in [
            json!({"action": "vote", "colour": "Blue"}),
            json!({"action": "vote", "colour": "gray"}),
            json!({"action": "vote", "colour": "ultraviolet"}),
            json!({"action": "vote", "colour": "blue", "extra": true}),
            json!({"action": "name", "text": "blue"}),
            json!({"action": "fetch"}),
            json!({"action": "argue", "text": "blue"}),
            json!({"action": "argue", "text": "blue", "cites": [], "colour": "blue"}),
        ] {
            assert!(!validator.is_valid(&value), "{value}");
            assert!(serde_json::from_value::<Action>(value).is_err());
        }
        for value in [
            json!({"action": "argue", "text": "🦜".repeat(ARGUMENT_CHARS + 1), "cites": []}),
            json!({"action": "argue", "text": "blue", "cites": vec!["id"; MAX_CITATIONS + 1]}),
        ] {
            assert!(!validator.is_valid(&value), "{value}");
        }

        let bare = argue("blue", vec![]);
        let bare_value = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            bare_value,
            json!({"action": "argue", "text": "blue", "cites": []})
        );
        assert!(bare_value.get("reply_to").is_none());
        let other = argument_id(&"ab".repeat(32));
        let reply = Action::Argue {
            text: "blue".into(),
            cites: vec![],
            reply_to: Some(other.clone()),
        };
        let reply_value = serde_json::to_value(&reply).unwrap();
        assert_eq!(reply_value["reply_to"], other);
        assert_eq!(
            serde_json::from_value::<Action>(reply_value).unwrap(),
            reply
        );
        assert!(
            validator.is_valid(
                &json!({"action": "argue", "text": "blue", "cites": [], "reply_to": other})
            )
        );
        assert!(validator.is_valid(&json!({"action": "argue", "text": "blue", "cites": []})));
        for reply_to in [
            format!("r:e:{}", "ab".repeat(32)),
            format!("r:a:{}", "a".repeat(63)),
            format!("r:a:{}", "AB".repeat(32)),
        ] {
            let value =
                json!({"action": "argue", "text": "blue", "cites": [], "reply_to": reply_to});
            assert!(!validator.is_valid(&value), "{value}");
        }
    }

    #[test]
    fn a_reply_names_another_authors_argument_in_this_run() {
        let other = argument_id(&"ab".repeat(32));
        let own = argument_id(&"cd".repeat(32));
        let vote = argument_id(&"ef".repeat(32));
        let evidence_id = format!("r:e:{}", "12".repeat(32));
        let arguments = [(other.clone(), "b".into()), (own.clone(), "a".into())];
        let other_records = [vote.clone(), evidence_id.clone()];
        let ctx = Context {
            author: "a",
            run: "today",
            evidence: &[],
            fetches_this_activation: 0,
            arguments: &arguments,
            other_records: &other_records,
            stance: None,
        };
        assert_eq!(
            validate(
                &Action::Argue {
                    text: "yes".into(),
                    cites: vec![],
                    reply_to: Some(other.clone()),
                },
                &ctx
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                &Action::Argue {
                    text: "yes".into(),
                    cites: vec![],
                    reply_to: Some(argument_id(&"99".repeat(32))),
                },
                &ctx
            ),
            Err(Refusal::UnknownReply)
        );
        for id in [vote, evidence_id] {
            assert_eq!(
                validate(
                    &Action::Argue {
                        text: "yes".into(),
                        cites: vec![],
                        reply_to: Some(id),
                    },
                    &ctx
                ),
                Err(Refusal::ReplyToNonArgument)
            );
        }
        assert_eq!(
            validate(
                &Action::Argue {
                    text: "yes".into(),
                    cites: vec![],
                    reply_to: Some(own),
                },
                &ctx
            ),
            Err(Refusal::SelfReply)
        );
        let records = [Evidence::new(
            "b".into(),
            "today".into(),
            Url::parse(WIKI).unwrap(),
            0,
            "fact",
        )];
        let foreign = Context {
            evidence: &records,
            ..ctx
        };
        assert_eq!(
            validate(
                &Action::Argue {
                    text: "yes".into(),
                    cites: vec![records[0].id().clone()],
                    reply_to: Some(other),
                },
                &foreign
            ),
            Err(Refusal::ForeignCitation)
        );
    }

    #[test]
    fn seed_uses_the_full_big_endian_digest_and_spreads_peers() {
        assert_eq!(seed_offset("abc", 0), 0);
        assert_eq!(seed_offset("abc", 1), 0);
        // Independent known SHA-256(abc) reduced modulo 1000.
        assert_eq!(seed_offset("abc", 1000), 965);
        let mut counts = [0; 20];
        for peer in (0..100).map(|i| format!("peer-{i}")) {
            let offset = seed_offset(&peer, counts.len());
            assert_eq!(seed_offset(&peer, counts.len()), offset);
            counts[offset] += 1;
            assert!(seed_offset(&peer, usize::MAX) < usize::MAX);
        }
        assert!(counts.into_iter().all(|count| count > 0));
    }

    fn stance(
        prefer: Vec<Colour>,
        accept: Vec<Colour>,
        refuse: Vec<Colour>,
        cites: Vec<EvidenceId>,
    ) -> Action {
        Action::Stance {
            prefer,
            accept,
            refuse,
            cites,
        }
    }

    #[test]
    fn stances_check_bounds_repeats_and_own_citations_in_order() {
        use Colour::{Blue, Green, Red, Teal};
        let evidence = [("a", "today"), ("b", "today"), ("a", "yesterday")]
            .map(|(a, r)| Evidence::new(a.into(), r.into(), Url::parse(WIKI).unwrap(), 0, "fact"));
        let ctx = context(&evidence, 0);
        let own = vec![evidence[0].id().clone()];
        for (action, expected) in [
            (
                stance(vec![Green], vec![Teal, Blue], vec![], vec![]),
                Ok(()),
            ),
            (stance(vec![Green], vec![], vec![Red], own.clone()), Ok(())),
            (
                stance(vec![], vec![], vec![], vec![]),
                Err(Refusal::StanceBounds),
            ),
            (
                stance(
                    vec![Green; 4],
                    vec![],
                    vec![],
                    vec![EvidenceId("bad".into())],
                ),
                Err(Refusal::StanceBounds),
            ),
            (
                stance(vec![Green], vec![Teal; 5], vec![], vec![]),
                Err(Refusal::StanceBounds),
            ),
            (
                stance(vec![Green], vec![], vec![Red; 4], vec![]),
                Err(Refusal::StanceBounds),
            ),
            (
                stance(vec![Green], vec![Green], vec![], vec![]),
                Err(Refusal::StanceColourRepeated),
            ),
            (
                stance(vec![Green, Green], vec![], vec![], vec![]),
                Err(Refusal::StanceColourRepeated),
            ),
            (
                stance(vec![Green], vec![], vec![Red], vec![]),
                Err(Refusal::StanceRefusalUncited),
            ),
            (
                stance(vec![Green], vec![], vec![], vec![own[0].clone(); 9]),
                Err(Refusal::TooManyCitations),
            ),
            (
                stance(
                    vec![Green],
                    vec![],
                    vec![Red],
                    vec![evidence[1].id().clone()],
                ),
                Err(Refusal::ForeignCitation),
            ),
            (
                stance(vec![Green], vec![], vec![], vec![evidence[2].id().clone()]),
                Err(Refusal::UnknownCitation),
            ),
            (
                stance(
                    vec![Green],
                    vec![],
                    vec![],
                    vec![EvidenceId("unknown".into())],
                ),
                Err(Refusal::UnknownCitation),
            ),
        ] {
            assert_eq!(validate(&action, &ctx), expected, "{action:?}");
        }
    }

    #[test]
    fn votes_need_reading_then_a_stance_and_only_its_choices() {
        use Colour::{Blue, Green, Navy, Red, Teal};
        let evidence = [Evidence::new(
            "a".into(),
            "today".into(),
            Url::parse(WIKI).unwrap(),
            0,
            "fact",
        )];
        let arguments = [("arg".into(), "a".into())];
        let mut ctx = context(&evidence, 0);
        ctx.arguments = &arguments;
        assert_eq!(
            validate(&Action::Vote { colour: Blue }, &ctx),
            Err(Refusal::VoteBeforeStance)
        );
        let stance = tally::Stance {
            id: "stance".into(),
            author: "a".into(),
            at_ms: 0,
            prefer: vec![Green],
            accept: vec![Teal, Blue],
            refuse: vec![Red],
            cites: vec![],
        };
        ctx.stance = Some(&stance);
        for colour in [Green, Teal, Blue] {
            assert_eq!(validate(&Action::Vote { colour }, &ctx), Ok(()));
        }
        for colour in [Red, Navy] {
            assert_eq!(
                validate(&Action::Vote { colour }, &ctx),
                Err(Refusal::VoteOutsideStance)
            );
        }
        ctx.evidence = &[];
        assert_eq!(
            validate(&Action::Vote { colour: Blue }, &ctx),
            Err(Refusal::VoteBeforeReading)
        );
        ctx.evidence = &evidence;
        ctx.arguments = &[];
        assert_eq!(
            validate(&Action::Vote { colour: Blue }, &ctx),
            Err(Refusal::VoteBeforeReading)
        );
        for (refusal, reason) in [
            (
                Refusal::StanceBounds,
                "prefer needs 1 to 3 colours, accept 0 to 4, refuse 0 to 3",
            ),
            (
                Refusal::StanceColourRepeated,
                "name each colour at most once across prefer, accept and refuse",
            ),
            (
                Refusal::StanceRefusalUncited,
                "a refusal cites at least one evidence id you fetched this run; cite one or leave refuse empty",
            ),
            (
                Refusal::VoteBeforeStance,
                "declare a stance first; a vote needs one fetch, a stance and one argument in this run",
            ),
            (
                Refusal::VoteOutsideStance,
                "vote a colour in your current prefer or accept lists, or declare a new stance first",
            ),
        ] {
            assert_eq!(refusal.reason(), Some(reason));
        }
    }

    #[test]
    fn stance_schema_bounds_vocabulary_required_fields_and_rust_repeat_check() {
        let schema = schema();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert_eq!(
            schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["properties"]["action"]["const"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["fetch", "argue", "stance", "vote"]
        );
        let good = json!({"action":"stance", "prefer":["blue","green","teal"], "accept":["navy","red","sky","cyan"], "refuse":["black","white","pink"], "cites":vec!["id";MAX_CITATIONS]});
        assert!(validator.is_valid(&good));
        let action: Action = serde_json::from_value(good.clone()).unwrap();
        assert_eq!(serde_json::to_value(action).unwrap(), good);
        for (key, value) in [
            ("prefer", json!([])),
            ("prefer", json!(vec!["blue"; 4])),
            ("accept", json!(vec!["blue"; 5])),
            ("refuse", json!(vec!["blue"; 4])),
            ("prefer", json!(["Blue"])),
            ("prefer", json!(["ultraviolet"])),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(!validator.is_valid(&bad), "{bad}");
            if let Ok(action) = serde_json::from_value::<Action>(bad) {
                assert!(validate(&action, &context(&[], 0)).is_err());
            }
        }
        for key in ["prefer", "accept", "refuse", "cites"] {
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(!validator.is_valid(&bad));
            assert!(serde_json::from_value::<Action>(bad).is_err());
        }
        let mut bad = good.clone();
        bad["extra"] = json!(true);
        assert!(!validator.is_valid(&bad));
        assert!(serde_json::from_value::<Action>(bad).is_err());
        let repeated =
            json!({"action":"stance","prefer":["blue","blue"],"accept":[],"refuse":[],"cites":[]});
        assert!(validator.is_valid(&repeated));
        assert_eq!(
            validate(&serde_json::from_value(repeated).unwrap(), &context(&[], 0)),
            Err(Refusal::StanceColourRepeated)
        );
    }
}
