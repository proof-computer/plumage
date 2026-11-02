//! Plumage's records on the generic store envelope.

use crate::actions::{
    ARGUMENT_CHARS, Action, MAX_CITATIONS, STANCE_ACCEPT_MAX, STANCE_PREFER_MAX, STANCE_PREFER_MIN,
    STANCE_REFUSE_MAX,
};
use crate::evidence::{Allowlist, Evidence};
use forthebirds_store::{ActionRecord, AppAction, EvidenceRecord, LedgerRecord, check_record};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;
/// An argue that names `reply_to`. Every other record stays at [`SCHEMA`].
pub const REPLY_SCHEMA: u32 = 2;
/// A stance. Every other record stays at [`SCHEMA`] or [`REPLY_SCHEMA`].
pub const STANCE_SCHEMA: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Record {
    Action(ActionRecord<Action>),
    Evidence(EvidenceRecord<Evidence>),
}

impl From<ActionRecord<Action>> for Record {
    fn from(record: ActionRecord<Action>) -> Self {
        Self::Action(record)
    }
}

impl From<EvidenceRecord<Evidence>> for Record {
    fn from(record: EvidenceRecord<Evidence>) -> Self {
        Self::Evidence(record)
    }
}

impl TryFrom<Record> for ActionRecord<Action> {
    type Error = forthebirds_store::Error;

    fn try_from(record: Record) -> Result<Self, Self::Error> {
        match record {
            Record::Action(record) => Ok(record),
            Record::Evidence(_) => Err(forthebirds_store::Error::new(
                "storage",
                "wrong action type",
            )),
        }
    }
}

impl TryFrom<Record> for EvidenceRecord<Evidence> {
    type Error = forthebirds_store::Error;

    fn try_from(record: Record) -> Result<Self, Self::Error> {
        match record {
            Record::Evidence(record) => Ok(record),
            Record::Action(_) => Err(forthebirds_store::Error::new(
                "storage",
                "wrong evidence type",
            )),
        }
    }
}

impl AppAction for Action {
    fn validate_shape(&self) -> forthebirds_store::Result<()> {
        match self {
            Action::Fetch { url } => {
                if !Allowlist::permits(url) {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "fetch url is outside the allowlist",
                    ));
                }
            }
            Action::Argue {
                text,
                cites,
                reply_to,
            } => {
                if text.chars().count() > ARGUMENT_CHARS {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "argument exceeds 400 characters",
                    ));
                }
                if cites.len() > MAX_CITATIONS {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "argument cites more than 8 evidence records",
                    ));
                }
                if cites.iter().any(|id| !is_lower_hex(&id.0, 64)) {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "citation must be 64 lowercase hex digits",
                    ));
                }
                if let Some(target) = reply_to
                    && !is_argument_id(target)
                {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "reply_to must be r:a: and 64 lowercase hex digits",
                    ));
                }
            }
            Action::Stance {
                prefer,
                accept,
                refuse,
                cites,
            } => {
                let invalid = |message| forthebirds_store::Error::new("invalid_action", message);
                if !(STANCE_PREFER_MIN..=STANCE_PREFER_MAX).contains(&prefer.len())
                    || accept.len() > STANCE_ACCEPT_MAX
                    || refuse.len() > STANCE_REFUSE_MAX
                {
                    return Err(invalid(
                        "stance prefer needs 1 to 3 colours, accept at most 4, refuse at most 3",
                    ));
                }
                let mut seen = std::collections::BTreeSet::new();
                if prefer
                    .iter()
                    .chain(accept)
                    .chain(refuse)
                    .any(|colour| !seen.insert(colour))
                {
                    return Err(invalid("stance names a colour more than once"));
                }
                if cites.len() > MAX_CITATIONS {
                    return Err(invalid("stance cites more than 8 evidence records"));
                }
                if !refuse.is_empty() && cites.is_empty() {
                    return Err(invalid("stance refusal cites no evidence"));
                }
                if cites.iter().any(|id| !is_lower_hex(&id.0, 64)) {
                    return Err(invalid("citation must be 64 lowercase hex digits"));
                }
            }
            Action::Vote { .. } => {}
        }
        Ok(())
    }

    fn evidence_ids(&self) -> Vec<&str> {
        match self {
            Action::Argue { cites, .. } | Action::Stance { cites, .. } => {
                cites.iter().map(|id| id.0.as_str()).collect()
            }
            Action::Fetch { .. } | Action::Vote { .. } => Vec::new(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Action::Fetch { .. } => "fetch",
            Action::Argue { .. } => "argue",
            Action::Stance { .. } => "stance",
            Action::Vote { .. } => "vote",
        }
    }

    fn is_content(&self) -> bool {
        matches!(self, Action::Argue { .. })
    }
}

impl LedgerRecord for Record {
    const APP: &'static str = "plumage";
    const MAX_KNOWN_SCHEMA: u32 = 3;

    fn id(&self) -> &str {
        match self {
            Self::Action(record) => &record.id,
            Self::Evidence(record) => &record.id,
        }
    }

    fn author(&self) -> &str {
        match self {
            Self::Action(record) => &record.author,
            Self::Evidence(record) => &record.observed_by,
        }
    }

    fn schema(&self) -> u32 {
        match self {
            Self::Action(record) => record.schema,
            Self::Evidence(record) => record.schema,
        }
    }

    fn validate(&self, run: &str) -> forthebirds_store::Result<()> {
        check_record(self)?;
        match self {
            Self::Action(record) => {
                let reply = matches!(
                    &record.action,
                    Action::Argue {
                        reply_to: Some(_),
                        ..
                    }
                );
                let expected = if matches!(record.action, Action::Stance { .. }) {
                    STANCE_SCHEMA
                } else if reply {
                    REPLY_SCHEMA
                } else {
                    SCHEMA
                };
                if record.schema != expected {
                    return Err(forthebirds_store::Error::new(
                        "invalid",
                        "schema does not match the record",
                    ));
                }
                record.validate_envelope(run)?;
                if matches!(record.action, Action::Fetch { .. }) {
                    return Err(forthebirds_store::Error::new(
                        "invalid_action",
                        "a fetch is recorded as evidence",
                    ));
                }
                Ok(())
            }
            Self::Evidence(record) => {
                if record.schema != SCHEMA {
                    return Err(forthebirds_store::Error::new(
                        "invalid",
                        "schema does not match the record",
                    ));
                }
                record.validate_envelope(run)?;
                if record.evidence.run() != record.run_id
                    || record.evidence.author() != record.observed_by
                {
                    return Err(forthebirds_store::Error::new(
                        "conflict",
                        "evidence author or run does not match the record",
                    ));
                }
                Ok(())
            }
        }
    }

    fn check_id(id: &str) -> forthebirds_store::Result<()> {
        let bytes = id.as_bytes();
        let prefixed = bytes.len() == 68 && (id.starts_with("r:a:") || id.starts_with("r:e:"));
        if prefixed && bytes[4..].iter().copied().all(is_hex_byte) {
            Ok(())
        } else {
            Err(forthebirds_store::Error::new(
                "invalid",
                "record id must be r:a: or r:e: and 64 lowercase hex digits",
            ))
        }
    }

    fn receipt(&self) -> Option<(u64, u64)> {
        match self {
            Self::Action(record) => Some((record.author_sequence, record.memory_tokens)),
            Self::Evidence(_) => None,
        }
    }
}

fn is_argument_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 68 && id.starts_with("r:a:") && bytes[4..].iter().copied().all(is_hex_byte)
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(is_hex_byte)
}

fn is_hex_byte(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::Colour;
    use forthebirds_store::{ChangesPage, LedgerRecord, action_id};
    use serde_json::{Value, json};

    fn vote_record() -> Record {
        Record::Action(ActionRecord {
            id: format!("r:a:{}", "ab".repeat(32)),
            schema: SCHEMA,
            run_id: "run-1".into(),
            author: "peer-a".into(),
            operation_id: "vote-1".into(),
            author_sequence: 1,
            created_at_ms: 5,
            action: Action::Vote {
                colour: Colour::Blue,
            },
            memory_tokens: 1,
            memory_remaining: 99,
        })
    }

    #[test]
    fn record_is_adjacently_tagged_and_a_newer_schema_is_skipped() {
        let record = vote_record();
        let good = serde_json::to_value(&record).unwrap();
        assert_eq!(good["kind"], "action");
        assert!(good["data"].is_object());
        assert_eq!(good["data"]["schema"], 1);
        let skipped = json!({"kind": "evidence", "data": {"schema": 4}});
        let page = json!({
            "database_id": "db-1",
            "records": [good, skipped],
            "last_seq": 1,
            "has_more": false,
        });
        let decoded: ChangesPage<Record> = serde_json::from_value(page).unwrap();
        assert_eq!(decoded.records.len(), 1);
        assert_eq!(decoded.records[0].id(), record.id());
        assert!(decoded.records[0].id().starts_with("r:a:"));
    }

    fn argue_record(schema: u32, reply_to: Option<String>) -> Record {
        let run = "run-1";
        let author = "peer-a";
        let operation = "argue-1";
        Record::Action(ActionRecord {
            id: action_id(run, author, operation),
            schema,
            run_id: run.into(),
            author: author.into(),
            operation_id: operation.into(),
            author_sequence: 1,
            created_at_ms: 5,
            action: Action::Argue {
                text: "blue".into(),
                cites: vec![],
                reply_to,
            },
            memory_tokens: 1,
            memory_remaining: 99,
        })
    }

    /// A peer whose `MAX_KNOWN_SCHEMA` is still 1. `decode_page` skips a reply
    /// before this type has to understand it.
    #[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
    struct OldPeer {
        data: Value,
    }

    impl LedgerRecord for OldPeer {
        const APP: &'static str = "plumage";
        const MAX_KNOWN_SCHEMA: u32 = 1;

        fn id(&self) -> &str {
            self.data
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        }

        fn author(&self) -> &str {
            ""
        }

        fn schema(&self) -> u32 {
            self.data.get("schema").and_then(Value::as_u64).unwrap_or(0) as u32
        }

        fn validate(&self, _run: &str) -> forthebirds_store::Result<()> {
            Ok(())
        }

        fn check_id(_id: &str) -> forthebirds_store::Result<()> {
            Ok(())
        }

        fn receipt(&self) -> Option<(u64, u64)> {
            None
        }
    }

    #[test]
    fn a_reply_is_schema_2_and_an_old_peer_skips_it() {
        let target = format!("r:a:{}", "cd".repeat(32));
        let reply = argue_record(REPLY_SCHEMA, Some(target));
        assert_eq!(reply.schema(), REPLY_SCHEMA);
        reply.validate("run-1").unwrap();

        let at_one = argue_record(SCHEMA, Some(format!("r:a:{}", "cd".repeat(32))));
        assert_eq!(at_one.validate("run-1").unwrap_err().code, "invalid");

        let vote_at_two = {
            let mut vote = vote_record();
            match &mut vote {
                Record::Action(record) => record.schema = REPLY_SCHEMA,
                Record::Evidence(_) => unreachable!(),
            }
            vote
        };
        assert_eq!(vote_at_two.validate("run-1").unwrap_err().code, "invalid");

        let malformed = argue_record(REPLY_SCHEMA, Some(format!("r:e:{}", "ab".repeat(32))));
        assert_eq!(
            malformed.validate("run-1").unwrap_err().code,
            "invalid_action"
        );

        let kept = vote_record();
        let page = json!({
            "database_id": "db-1",
            "records": [
                serde_json::to_value(&kept).unwrap(),
                serde_json::to_value(&reply).unwrap(),
            ],
            "last_seq": 2,
            "has_more": false,
        });
        let current: ChangesPage<Record> = serde_json::from_value(page.clone()).unwrap();
        assert_eq!(current.records.len(), 2);
        let old: ChangesPage<OldPeer> = serde_json::from_value(page).unwrap();
        assert_eq!(old.records.len(), 1);
        assert_eq!(old.records[0].id(), kept.id());
    }

    fn stance_action() -> Action {
        Action::Stance {
            prefer: vec![Colour::Blue],
            accept: vec![],
            refuse: vec![Colour::Red],
            cites: vec![crate::evidence::EvidenceId("ab".repeat(32))],
        }
    }

    fn stance_record(action: Action, schema: u32) -> Record {
        let Record::Action(mut record) = vote_record() else {
            unreachable!()
        };
        record.action = action;
        record.id = action_id(&record.run_id, &record.author, &record.operation_id);
        record.schema = schema;
        Record::Action(record)
    }

    #[test]
    fn stance_schema_and_shape_are_validated_before_storage() {
        let action = stance_action();
        assert_eq!(action.kind(), "stance");
        assert!(!action.is_content());
        assert_eq!(action.evidence_ids(), vec!["ab".repeat(32).as_str()]);
        stance_record(action.clone(), STANCE_SCHEMA)
            .validate("run-1")
            .unwrap();
        assert_eq!(
            stance_record(action.clone(), SCHEMA)
                .validate("run-1")
                .unwrap_err()
                .code,
            "invalid"
        );
        let Record::Action(vote) = vote_record() else {
            unreachable!()
        };
        assert_eq!(
            stance_record(vote.action, STANCE_SCHEMA)
                .validate("run-1")
                .unwrap_err()
                .code,
            "invalid"
        );
        for (field, value, message) in [
            (
                "prefer",
                json!([]),
                "stance prefer needs 1 to 3 colours, accept at most 4, refuse at most 3",
            ),
            (
                "accept",
                json!(["blue"]),
                "stance names a colour more than once",
            ),
            ("cites", json!([]), "stance refusal cites no evidence"),
            (
                "cites",
                json!(["a".repeat(63)]),
                "citation must be 64 lowercase hex digits",
            ),
            (
                "cites",
                json!(vec!["ab".repeat(32); 9]),
                "stance cites more than 8 evidence records",
            ),
        ] {
            let mut value_action = serde_json::to_value(&action).unwrap();
            value_action[field] = value;
            let error = stance_record(serde_json::from_value(value_action).unwrap(), STANCE_SCHEMA)
                .validate("run-1")
                .unwrap_err();
            assert_eq!(error.code, "invalid_action");
            assert_eq!(error.message, message);
        }
    }

    #[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
    struct ReplyPeer {
        data: Value,
    }
    impl LedgerRecord for ReplyPeer {
        const APP: &'static str = "plumage";
        const MAX_KNOWN_SCHEMA: u32 = 2;
        fn id(&self) -> &str {
            self.data["id"].as_str().unwrap()
        }
        fn author(&self) -> &str {
            ""
        }
        fn schema(&self) -> u32 {
            self.data["schema"].as_u64().unwrap() as u32
        }
        fn validate(&self, _run: &str) -> forthebirds_store::Result<()> {
            Ok(())
        }
        fn check_id(_id: &str) -> forthebirds_store::Result<()> {
            Ok(())
        }
        fn receipt(&self) -> Option<(u64, u64)> {
            None
        }
    }

    #[test]
    fn old_peers_skip_stances_and_keep_the_records_they_understand() {
        let page = json!({"database_id":"db", "last_seq":3,"has_more":false,"records":[
            vote_record(), argue_record(REPLY_SCHEMA,Some(format!("r:a:{}","cd".repeat(32)))), stance_record(stance_action(),STANCE_SCHEMA)
        ]});
        let current: ChangesPage<Record> = serde_json::from_value(page.clone()).unwrap();
        assert_eq!(current.records.len(), 3);
        let old: ChangesPage<OldPeer> = serde_json::from_value(page.clone()).unwrap();
        assert_eq!(old.records.len(), 1);
        let reply: ChangesPage<ReplyPeer> = serde_json::from_value(page).unwrap();
        assert_eq!(reply.records.len(), 2);
    }
}
