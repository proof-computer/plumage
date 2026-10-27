//! Plumage's records on the generic store envelope.

use crate::actions::{ARGUMENT_CHARS, Action, MAX_CITATIONS};
use crate::evidence::{Allowlist, Evidence};
use forthebirds_store::{ActionRecord, AppAction, EvidenceRecord, LedgerRecord, check_record};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

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
            Action::Argue { text, cites } => {
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
            }
            Action::Vote { .. } => {}
        }
        Ok(())
    }

    fn evidence_ids(&self) -> Vec<&str> {
        match self {
            Action::Argue { cites, .. } => cites.iter().map(|id| id.0.as_str()).collect(),
            Action::Fetch { .. } | Action::Vote { .. } => Vec::new(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Action::Fetch { .. } => "fetch",
            Action::Argue { .. } => "argue",
            Action::Vote { .. } => "vote",
        }
    }

    fn is_content(&self) -> bool {
        matches!(self, Action::Argue { .. })
    }
}

impl LedgerRecord for Record {
    const APP: &'static str = "plumage";
    const MAX_KNOWN_SCHEMA: u32 = 1;

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
        if self.schema() != SCHEMA {
            return Err(forthebirds_store::Error::new("invalid", "schema is not 1"));
        }
        match self {
            Self::Action(record) => {
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
    use forthebirds_store::{ChangesPage, LedgerRecord};
    use serde_json::json;

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
        let skipped = json!({"kind": "evidence", "data": {"schema": 2}});
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
}
