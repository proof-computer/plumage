//! One peer's ledger and the day's evidence, arguments and votes.

use crate::actions::{self, Action, Context, Refusal};
use crate::closing;
use crate::error::{Error, Result};
use crate::evidence::{Evidence, EvidenceId};
use crate::record::{REPLY_SCHEMA, Record, SCHEMA};
use forthebirds_store::{ActionRecord, EvidenceRecord, Host, Ledger, LedgerConfig, LedgerRecord};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DayIndex {
    pub evidence: BTreeMap<String, (String, Evidence)>,
    pub arguments: Vec<Argument>,
    pub votes: Vec<closing::Vote>,
    pub latest_ms: u64,
    /// Ledger record ids already applied. Votes do not carry that id, so a
    /// repeat of the same record cannot be recognised from the vote alone.
    seen: BTreeSet<String>,
    /// Parallel to `votes`, giving each vote the total order of its record id.
    vote_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Argument {
    pub id: String,
    pub author: String,
    pub at_ms: u64,
    pub text: String,
    pub cites: Vec<EvidenceId>,
    pub reply_to: Option<String>,
}

impl DayIndex {
    /// Argument ids with their authors, and every other record id in this day.
    ///
    /// `seen` keeps a ledger id even when a repeated fetch is not the evidence
    /// row the index shows, so a reply can name that record and still be refused
    /// as something other than an argument.
    pub fn reply_records(&self) -> (Vec<(String, String)>, Vec<String>) {
        let arguments: Vec<(String, String)> = self
            .arguments
            .iter()
            .map(|argument| (argument.id.clone(), argument.author.clone()))
            .collect();
        let argument_ids: BTreeSet<&str> = arguments.iter().map(|(id, _)| id.as_str()).collect();
        let other_records = self
            .seen
            .iter()
            .filter(|id| !argument_ids.contains(id.as_str()))
            .cloned()
            .collect();
        (arguments, other_records)
    }
}

#[derive(Clone)]
pub struct Book {
    ledger: Ledger<Record, ()>,
    index: Arc<Mutex<DayIndex>>,
}

impl Book {
    pub async fn open(config: LedgerConfig) -> Result<Self> {
        let ledger = Ledger::open(config).await?;
        let book = Self {
            ledger,
            index: Arc::new(Mutex::new(DayIndex::default())),
        };
        let cache = book.ledger.recover(&book).await?;
        *book.ledger.cache.lock().await = cache;
        Ok(book)
    }

    pub fn ledger(&self) -> &Ledger<Record, ()> {
        &self.ledger
    }

    pub fn index(&self) -> DayIndex {
        self.lock_index().clone()
    }

    pub async fn save_evidence(&self, evidence: Evidence) -> Result<EvidenceRecord<Evidence>> {
        if evidence.author() != self.ledger.config.peer_id
            || evidence.run() != self.ledger.config.run_id
        {
            return Err(Error::new(
                "conflict",
                "evidence author or run is not this ledger's",
            ));
        }
        let record = self.ledger.save_evidence(self, evidence).await?;
        self.remember(&Record::from(record.clone()));
        Ok(record)
    }

    pub async fn write(
        &self,
        operation: &str,
        action: Action,
        memory_tokens: u64,
    ) -> Result<ActionRecord<Action>> {
        if matches!(action, Action::Fetch { .. }) {
            return Err(Error::new(
                "invalid_action",
                "a fetch is recorded as evidence",
            ));
        }
        if let Action::Argue { .. } = &action {
            let evidence = self.indexed_evidence();
            let (arguments, other_records) = self.lock_index().reply_records();
            let context = Context {
                author: &self.ledger.config.peer_id,
                run: &self.ledger.config.run_id,
                evidence: &evidence,
                fetches_this_activation: 0,
                arguments: &arguments,
                other_records: &other_records,
            };
            actions::validate(&action, &context).map_err(refused)?;
        }
        let schema = match &action {
            Action::Argue {
                reply_to: Some(_), ..
            } => REPLY_SCHEMA,
            _ => SCHEMA,
        };
        let record = self
            .ledger
            .execute(self, operation, action, memory_tokens, schema)
            .await?;
        self.remember(&Record::from(record.clone()));
        Ok(record)
    }

    fn indexed_evidence(&self) -> Vec<Evidence> {
        self.lock_index()
            .evidence
            .values()
            .map(|(_, evidence)| evidence.clone())
            .collect()
    }

    fn remember(&self, record: &Record) {
        remember(&mut self.lock_index(), record);
    }

    fn lock_index(&self) -> std::sync::MutexGuard<'_, DayIndex> {
        self.index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn refuse_imported_argue(&self, record: &ActionRecord<Action>) -> Result<()> {
        let Action::Argue { .. } = &record.action else {
            return Ok(());
        };
        let evidence = self.indexed_evidence();
        let (arguments, other_records) = self.lock_index().reply_records();
        let context = Context {
            author: &record.author,
            run: &record.run_id,
            evidence: &evidence,
            fetches_this_activation: 0,
            arguments: &arguments,
            other_records: &other_records,
        };
        if let Err(refusal) = actions::validate(&record.action, &context) {
            let code = if matches!(refusal, Refusal::UnknownCitation | Refusal::UnknownReply) {
                "missing_reference"
            } else {
                "invalid_action"
            };
            return Err(Error::new(code, format!("{refusal:?}")));
        }
        Ok(())
    }
}

impl Host<Record, ()> for Book {
    type Error = Error;

    fn ledger(&self) -> &Ledger<Record, ()> {
        &self.ledger
    }

    fn code(error: &Self::Error) -> &str {
        error.code
    }

    fn validate_import(
        &self,
        record: &Record,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let result = match record {
            Record::Action(action) => self.refuse_imported_argue(action),
            Record::Evidence(_) => Ok(()),
        };
        async move { result }
    }

    fn observe(&self, _app: &mut (), record: &Record) {
        self.remember(record);
    }

    fn recover_local(&self, _app: &mut (), _key: &str, _value: &Value) -> Result<()> {
        Ok(())
    }
}

fn refused(refusal: Refusal) -> Error {
    Error::new("refused", format!("{refusal:?}"))
}

fn remember(index: &mut DayIndex, record: &Record) {
    if !index.seen.insert(record.id().to_string()) {
        return;
    }
    match record {
        Record::Action(action) => {
            index.latest_ms = index.latest_ms.max(action.created_at_ms);
            match &action.action {
                Action::Argue {
                    text,
                    cites,
                    reply_to,
                } => {
                    let argument = Argument {
                        id: action.id.clone(),
                        author: action.author.clone(),
                        at_ms: action.created_at_ms,
                        text: text.clone(),
                        cites: cites.clone(),
                        reply_to: reply_to.clone(),
                    };
                    let position = index.arguments.partition_point(|existing| {
                        (existing.at_ms, existing.id.as_str())
                            <= (argument.at_ms, argument.id.as_str())
                    });
                    index.arguments.insert(position, argument);
                }
                Action::Vote { colour } => {
                    let vote = closing::Vote {
                        author: action.author.clone(),
                        colour: *colour,
                        at_ms: action.created_at_ms,
                    };
                    let position = index
                        .votes
                        .iter()
                        .zip(index.vote_ids.iter())
                        .position(|(existing, existing_id)| {
                            (
                                existing.at_ms,
                                existing.author.as_str(),
                                existing.colour,
                                existing_id.as_str(),
                            ) > (
                                vote.at_ms,
                                vote.author.as_str(),
                                vote.colour,
                                action.id.as_str(),
                            )
                        })
                        .unwrap_or(index.votes.len());
                    index.votes.insert(position, vote);
                    index.vote_ids.insert(position, action.id.clone());
                }
                Action::Fetch { .. } => {}
            }
        }
        Record::Evidence(evidence) => {
            index.latest_ms = index.latest_ms.max(evidence.observed_at_ms);
            // The bare id omits fetched_at_ms, so one citation can have several
            // ledger records. Recovery walks ledger-id order and a live write
            // walks commit order; keep the earliest fetch, then the smaller
            // ledger id, so both rebuild the same entry.
            let bare_id = evidence.evidence.id().0.clone();
            let incoming = (evidence.evidence.fetched_at_ms(), evidence.id.as_str());
            let keep_incoming = match index.evidence.get(&bare_id) {
                Some((kept_id, kept)) => incoming < (kept.fetched_at_ms(), kept_id.as_str()),
                None => true,
            };
            if keep_incoming {
                index
                    .evidence
                    .insert(bare_id, (evidence.id.clone(), evidence.evidence.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ARGUMENT_CHARS;
    use crate::colour::Colour;
    use forthebirds_store::{LedgerConfig, LedgerRecord, action_id};
    use std::path::Path;
    use url::Url;

    fn config(path: &Path, peer: &str) -> LedgerConfig {
        LedgerConfig {
            database_path: path.to_path_buf(),
            database_cache_bytes: 1024 * 1024,
            run_id: "run-1".into(),
            peer_id: peer.into(),
            token_budget: Some(100),
            memory_budget: 100,
            memory_item_tokens: 40,
            max_documents: 100,
        }
    }

    fn wiki() -> Url {
        Url::parse("https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/10/26").unwrap()
    }

    fn evidence_for(author: &str) -> Evidence {
        Evidence::new(
            author.into(),
            "run-1".into(),
            wiki(),
            1_700_000_000_000,
            "a fact about the day",
        )
    }

    async fn open_at(dir: &Path, name: &str, peer: &str) -> Book {
        Book::open(config(&dir.join(name), peer)).await.unwrap()
    }

    #[tokio::test]
    async fn saves_evidence_argument_and_vote_and_reopens_the_same_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("day.redb");
        let book = Book::open(config(&path, "peer-a")).await.unwrap();
        let evidence = evidence_for("peer-a");
        let saved = book.save_evidence(evidence.clone()).await.unwrap();
        let argument = book
            .write(
                "argue-1",
                Action::Argue {
                    text: "the day is blue".into(),
                    cites: vec![evidence.id().clone()],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        let vote = book
            .write(
                "vote-1",
                Action::Vote {
                    colour: Colour::Blue,
                },
                1,
            )
            .await
            .unwrap();

        let index = book.index();
        assert_eq!(index.evidence.len(), 1);
        assert_eq!(index.arguments.len(), 1);
        assert_eq!(index.votes.len(), 1);
        assert_eq!(index.arguments[0].cites, vec![evidence.id().clone()]);
        assert_eq!(index.votes[0].colour, Colour::Blue);
        let (ledger_id, stored) = index.evidence.values().next().unwrap();
        assert!(ledger_id.starts_with("r:e:"));
        assert_eq!(stored, &evidence);

        let page = book.ledger().page(None, 32).await.unwrap();
        let page_ids: Vec<_> = page
            .records
            .iter()
            .map(|record| record.id().to_string())
            .collect();
        let mut id_order = vec![saved.id.clone(), argument.id.clone(), vote.id.clone()];
        id_order.sort();
        assert_eq!(page_ids, id_order);
        assert!(page_ids[..2].iter().all(|id| id.starts_with("r:a:")));
        assert!(page_ids[2].starts_with("r:e:"));
        assert_ne!(
            page_ids,
            vec![saved.id.clone(), argument.id.clone(), vote.id]
        );

        drop(book);
        let reopened = Book::open(config(&path, "peer-a")).await.unwrap();
        assert_eq!(reopened.index(), index);
    }

    #[tokio::test]
    async fn repeated_fetch_keeps_the_earliest_evidence_in_every_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("day.redb");
        let book = Book::open(config(&path, "peer-a")).await.unwrap();
        let mut saved = Vec::new();
        for fetched_at_ms in [3, 1, 4, 2] {
            let evidence = Evidence::new(
                "peer-a".into(),
                "run-1".into(),
                wiki(),
                fetched_at_ms,
                "same feed",
            );
            saved.push(book.save_evidence(evidence).await.unwrap());
        }

        let live = book.index();
        assert_eq!(live.evidence.len(), 1);
        let (ledger_id, stored) = live.evidence.values().next().unwrap();
        let earliest = saved
            .iter()
            .find(|record| record.evidence.fetched_at_ms() == 1)
            .unwrap();
        assert_eq!(stored.fetched_at_ms(), 1);
        assert_eq!(ledger_id, &earliest.id);

        drop(book);
        let reopened = Book::open(config(&path, "peer-a")).await.unwrap();
        assert_eq!(reopened.index(), live);

        let by_id = reopened.ledger().page(None, 32).await.unwrap();
        let by_commit = reopened.ledger().changes(0, None).await.unwrap();
        for (name, records) in [("id", by_id.records), ("commit", by_commit.records)] {
            let reader = open_at(dir.path(), &format!("{name}.redb"), "peer-b").await;
            assert_eq!(
                reader.ledger().import(&reader, records).await.unwrap(),
                4,
                "{name}"
            );
            assert_eq!(reader.index(), reopened.index(), "{name}");
        }
    }

    #[tokio::test]
    async fn write_refuses_fetch_unknown_and_overlong_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_at(dir.path(), "peer.redb", "peer-a").await;
        let evidence = evidence_for("peer-a");
        book.save_evidence(evidence.clone()).await.unwrap();
        book.write(
            "argue-1",
            Action::Argue {
                text: "short".into(),
                cites: vec![evidence.id().clone()],
                reply_to: None,
            },
            1,
        )
        .await
        .unwrap();
        let before = book.index();

        let fetch = book
            .write("fetch-1", Action::Fetch { url: wiki() }, 1)
            .await
            .unwrap_err();
        assert_eq!(fetch.code, "invalid_action");

        let unknown = book
            .write(
                "argue-unknown",
                Action::Argue {
                    text: "missing".into(),
                    cites: vec![EvidenceId("ab".repeat(32))],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap_err();
        assert_eq!(unknown.code, "refused");
        assert!(unknown.message.contains("UnknownCitation"));

        let overlong = book
            .write(
                "argue-long",
                Action::Argue {
                    text: "x".repeat(ARGUMENT_CHARS + 1),
                    cites: vec![evidence.id().clone()],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap_err();
        assert_eq!(overlong.code, "refused");
        assert!(overlong.message.contains("TextTooLong"));
        assert_eq!(book.index(), before);

        let vote = Action::Vote {
            colour: Colour::Blue,
        };
        let first = book.write("vote-1", vote.clone(), 1).await.unwrap();
        let indexed = book.index();
        assert_eq!(indexed.votes.len(), 1);
        let second = book.write("vote-1", vote, 1).await.unwrap();
        assert_eq!(
            serde_json::to_value(&second).unwrap(),
            serde_json::to_value(&first).unwrap()
        );
        assert_eq!(book.index(), indexed);
    }

    #[tokio::test]
    async fn import_indexes_a_peers_day_and_refuses_bad_citations() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_at(dir.path(), "a.redb", "peer-a").await;
        let evidence = evidence_for("peer-a");
        author.save_evidence(evidence.clone()).await.unwrap();
        author
            .write(
                "argue-1",
                Action::Argue {
                    text: "because of this".into(),
                    cites: vec![evidence.id().clone()],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        author
            .write(
                "vote-1",
                Action::Vote {
                    colour: Colour::Blue,
                },
                1,
            )
            .await
            .unwrap();

        let reader = open_at(dir.path(), "b.redb", "peer-b").await;
        let argue_only = author
            .ledger()
            .changes(0, None)
            .await
            .unwrap()
            .records
            .into_iter()
            .find(|record| matches!(record, Record::Action(action) if matches!(action.action, Action::Argue { .. })))
            .unwrap();
        let missing = reader
            .ledger()
            .import(&reader, vec![argue_only])
            .await
            .unwrap_err();
        assert_eq!(missing.code, "missing_reference");
        assert!(reader.index().evidence.is_empty());

        let page = author.ledger().changes(0, None).await.unwrap();
        assert_eq!(
            reader.ledger().import(&reader, page.records).await.unwrap(),
            3
        );
        assert_eq!(reader.index(), author.index());

        let foreign_evidence = evidence_for("peer-b");
        reader
            .save_evidence(foreign_evidence.clone())
            .await
            .unwrap();
        let foreign = author
            .ledger()
            .execute(
                &author,
                "foreign",
                Action::Argue {
                    text: "your fact".into(),
                    cites: vec![foreign_evidence.id().clone()],
                    reply_to: None,
                },
                1,
                SCHEMA,
            )
            .await
            .unwrap();
        let foreign = reader
            .ledger()
            .import(&reader, vec![Record::from(foreign)])
            .await
            .unwrap_err();
        assert_eq!(foreign.code, "invalid_action");
        assert!(foreign.message.contains("ForeignCitation"));

        let operation = "own-vote";
        let own = Record::Action(ActionRecord {
            id: action_id("run-1", "peer-b", operation),
            schema: SCHEMA,
            run_id: "run-1".into(),
            author: "peer-b".into(),
            operation_id: operation.into(),
            author_sequence: 1,
            created_at_ms: 5,
            action: Action::Vote {
                colour: Colour::Red,
            },
            memory_tokens: 1,
            memory_remaining: 90,
        });
        let owned = reader
            .ledger()
            .import(&reader, vec![own])
            .await
            .unwrap_err();
        assert_eq!(owned.code, "conflict");
    }

    #[tokio::test]
    async fn a_written_reply_is_indexed_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_at(dir.path(), "b.redb", "peer-b").await;
        let evidence = evidence_for("peer-b");
        author.save_evidence(evidence.clone()).await.unwrap();
        let target = author
            .write(
                "argue-1",
                Action::Argue {
                    text: "the day is blue".into(),
                    cites: vec![evidence.id().clone()],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        assert_eq!(target.schema, SCHEMA);
        let page = author.ledger().changes(0, None).await.unwrap();

        let path = dir.path().join("a.redb");
        let writer = Book::open(config(&path, "peer-a")).await.unwrap();
        writer.ledger().import(&writer, page.records).await.unwrap();
        let own = evidence_for("peer-a");
        writer.save_evidence(own.clone()).await.unwrap();
        let reply = writer
            .write(
                "reply-1",
                Action::Argue {
                    text: "answering you".into(),
                    cites: vec![own.id().clone()],
                    reply_to: Some(target.id.clone()),
                },
                1,
            )
            .await
            .unwrap();
        assert_eq!(reply.schema, REPLY_SCHEMA);
        let indexed = writer.index();
        let stored = indexed
            .arguments
            .iter()
            .find(|argument| argument.id == reply.id)
            .unwrap();
        assert_eq!(stored.reply_to.as_deref(), Some(target.id.as_str()));
        let answered = indexed
            .arguments
            .iter()
            .find(|argument| argument.id == target.id)
            .unwrap();
        assert_eq!(answered.reply_to, None);

        drop(writer);
        let reopened = Book::open(config(&path, "peer-a")).await.unwrap();
        assert_eq!(reopened.index(), indexed);
    }

    #[tokio::test]
    async fn import_holds_a_reply_until_its_target_and_refuses_self_and_vote() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_at(dir.path(), "b.redb", "peer-b").await;
        let evidence = evidence_for("peer-b");
        let saved = author.save_evidence(evidence.clone()).await.unwrap();
        let target = author
            .write(
                "argue-1",
                Action::Argue {
                    text: "first".into(),
                    cites: vec![evidence.id().clone()],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        let vote = author
            .write(
                "vote-1",
                Action::Vote {
                    colour: Colour::Blue,
                },
                1,
            )
            .await
            .unwrap();
        let before_reply = author.ledger().changes(0, None).await.unwrap();
        let replier = open_at(dir.path(), "c.redb", "peer-c").await;
        let reply = replier
            .ledger()
            .execute(
                &replier,
                "reply-1",
                Action::Argue {
                    text: "answering".into(),
                    cites: vec![],
                    reply_to: Some(target.id.clone()),
                },
                1,
                REPLY_SCHEMA,
            )
            .await
            .unwrap();

        let reader = open_at(dir.path(), "a.redb", "peer-a").await;
        let early = reader
            .ledger()
            .import(&reader, vec![Record::from(reply.clone())])
            .await
            .unwrap_err();
        assert_eq!(early.code, "missing_reference");
        assert!(early.message.contains("UnknownReply"));
        assert!(reader.index().arguments.is_empty());

        let prelude: Vec<Record> = before_reply
            .records
            .into_iter()
            .filter(|record| record.id() == saved.id || record.id() == target.id)
            .collect();
        reader.ledger().import(&reader, prelude).await.unwrap();
        assert_eq!(
            reader
                .ledger()
                .import(&reader, vec![Record::from(reply)])
                .await
                .unwrap(),
            1
        );
        let stored = reader
            .index()
            .arguments
            .into_iter()
            .find(|argument| argument.reply_to.is_some())
            .unwrap();
        assert_eq!(stored.reply_to.as_deref(), Some(target.id.as_str()));

        let self_reply = author
            .ledger()
            .execute(
                &author,
                "self-reply",
                Action::Argue {
                    text: "myself".into(),
                    cites: vec![],
                    reply_to: Some(target.id.clone()),
                },
                1,
                REPLY_SCHEMA,
            )
            .await
            .unwrap();
        let self_reply = reader
            .ledger()
            .import(&reader, vec![Record::from(self_reply)])
            .await
            .unwrap_err();
        assert_eq!(self_reply.code, "invalid_action");
        assert!(self_reply.message.contains("SelfReply"));

        reader
            .ledger()
            .import(&reader, vec![Record::from(vote.clone())])
            .await
            .unwrap();
        let vote_reply = author
            .ledger()
            .execute(
                &author,
                "vote-reply",
                Action::Argue {
                    text: "not an argument".into(),
                    cites: vec![],
                    reply_to: Some(vote.id.clone()),
                },
                1,
                REPLY_SCHEMA,
            )
            .await
            .unwrap();
        let vote_reply = reader
            .ledger()
            .import(&reader, vec![Record::from(vote_reply)])
            .await
            .unwrap_err();
        assert_eq!(vote_reply.code, "invalid_action");
        assert!(vote_reply.message.contains("ReplyToNonArgument"));
    }
}
