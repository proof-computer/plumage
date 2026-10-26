//! Colour-domain action grammar and validation; fetching is only an intent.

use crate::{
    colour::Colour,
    evidence::{Allowlist, Evidence, EvidenceId},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

// Q-20261023-7xbm: three fetches per activation, seeded by peer-id SHA-256.
pub const FETCHES_PER_ACTIVATION: usize = 3;
pub const ARGUMENT_CHARS: usize = 400;
pub const MAX_CITATIONS: usize = 8;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Fetch {
        url: Url,
    },
    Argue {
        text: String,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    HostNotAllowed,
    BudgetExhausted,
    UnknownCitation,
    ForeignCitation,
    TextTooLong,
    TooManyCitations,
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
        Action::Argue { text, cites } => {
            if text.chars().count() > ARGUMENT_CHARS {
                return Err(Refusal::TextTooLong);
            }
            if cites.len() > MAX_CITATIONS {
                return Err(Refusal::TooManyCitations);
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
        Action::Vote { .. } => {}
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
                           "items": {"type": "string"}}}},
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

    const WIKI: &str = "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/10/23";

    fn context(evidence: &[Evidence], fetches: usize) -> Context<'_> {
        Context {
            author: "a",
            run: "today",
            evidence,
            fetches_this_activation: fetches,
        }
    }

    fn argue(text: &str, cites: Vec<EvidenceId>) -> Action {
        Action::Argue {
            text: text.into(),
            cites,
        }
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
        assert_eq!(
            validate(
                &Action::Vote {
                    colour: Colour::Blue
                },
                &context(&[], 3)
            ),
            Ok(())
        );
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
        let declared = &schema()["oneOf"][2]["properties"]["colour"]["enum"];
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
}
