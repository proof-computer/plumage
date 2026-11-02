//! One bounded activation: fetch, stance, argue, vote, then stop.

use forthebirds_inference::model::{Model, Prepared};
use forthebirds_runtime::activation::{Activation, Limits, Step};
use forthebirds_store::ActionRecord;
use serde_json::{Value, json};
use url::Url;

use crate::actions::{self, Action, FETCHES_PER_ACTIVATION};
use crate::book::Book;
use crate::clock::DayClock;
use crate::colour::Colour;
use crate::day::Day;
use crate::error::{Error, Result};
use crate::evidence::Evidence;
use crate::fetch::{self, Source};
use crate::prompt;

/// One fetch that returned [`crate::fetch::FetchError`].
///
/// `host` is the URL host only. The path, query, message and body stay out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchFailure {
    pub code: &'static str,
    pub host: String,
}

/// The host `forthebirds_runtime` drives for one peer activation.
pub struct PeerActivation<'a, S: Source> {
    book: &'a Book,
    model: &'a Model,
    source: &'a S,
    peer_id: &'a str,
    day: Day,
    clock: DayClock,
    completed_fetches: usize,
    fetch_failures: &'a mut Vec<FetchFailure>,
}

impl<'a, S: Source> Activation for PeerActivation<'a, S> {
    type Output = Colour;
    type Error = Error;

    async fn begin(&mut self, _activation: &str, _context: &mut Value) -> Result<()> {
        Ok(())
    }

    async fn before_turn(&mut self, _turn: usize, context: &mut Value) -> Result<Option<String>> {
        let index = self.book.index();
        let fetches_left = FETCHES_PER_ACTIVATION.saturating_sub(self.completed_fetches);
        let gate = prompt::vote_gate(&index, self.peer_id, &self.book.ledger().config.run_id);
        *context = prompt::context(
            &self.day,
            &self.clock,
            self.peer_id,
            &index,
            fetches_left,
            gate,
        );
        Ok(None)
    }

    async fn schema_for_turn(&mut self, _turn: usize, _context: &Value) -> Result<Option<Value>> {
        let run = &self.book.ledger().config.run_id;
        let index = self.book.index();
        let gate = prompt::vote_gate(&index, self.peer_id, run);
        let choices = gate
            .may_vote()
            .then(|| {
                index
                    .stances
                    .get(self.peer_id)
                    .map(crate::tally::Stance::vote_choices)
            })
            .flatten();
        Ok(Some(prompt::schema_allowing(
            &self.day,
            self.peer_id,
            choices.as_deref(),
        )))
    }

    async fn prepare(&mut self, messages: &mut Vec<Value>, context: &Value) -> Result<Prepared> {
        if messages.len() < 2 {
            return Err(Error::new(
                "invalid",
                "activation messages have no user turn",
            ));
        }
        let mut context = context.clone();
        loop {
            messages[1]["content"] = Value::String(context.to_string());
            match self.model.prepare_render(messages).await {
                Ok(prepared) => return Ok(prepared),
                Err(error) if error.code == "limit" => {
                    if !drop_oldest(&mut context) {
                        return Err(error.into());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    async fn reserve(
        &mut self,
        operation: &str,
        amount: u64,
        _continued: bool,
        _messages: &[Value],
    ) -> Result<String> {
        self.book
            .ledger()
            .reserve_inference(self.book, operation, amount)
            .await
    }

    async fn settle(&mut self, reservation: &str, actual: u64) -> Result<()> {
        self.book
            .ledger()
            .settle_inference(self.book, reservation, actual)
            .await
    }

    async fn act(
        &mut self,
        _activation: &str,
        operation: &str,
        call: &Value,
        _input_tokens: u64,
        _continued: bool,
        _context: &mut Value,
    ) -> Result<Step<Colour>> {
        let action: Action = match serde_json::from_value(call.clone()) {
            Ok(action) => action,
            Err(_) => return continue_with(json!({"error": "invalid_action"})),
        };
        let run = self.book.ledger().config.run_id.clone();
        let own = own_evidence(self.book, self.peer_id, &run);
        let index = self.book.index();
        let (arguments, other_records) = index.reply_records();
        let check = actions::Context {
            author: self.peer_id,
            run: &run,
            evidence: &own,
            fetches_this_activation: self.completed_fetches,
            arguments: &arguments,
            other_records: &other_records,
            stance: index.stances.get(self.peer_id),
        };
        if let Err(refusal) = actions::validate(&action, &check) {
            let mut result = json!({"refused": format!("{refusal:?}")});
            if let Some(reason) = refusal.reason() {
                result["reason"] = json!(reason);
            }
            return continue_with(result);
        }
        match action {
            Action::Fetch { url } => self.fetch(url).await,
            Action::Argue { .. } => self.argue(operation, action).await,
            Action::Stance { .. } => self.stance(operation, action).await,
            Action::Vote { colour } => self.vote(operation, action, colour).await,
        }
    }
}

impl<'a, S: Source> PeerActivation<'a, S> {
    async fn fetch(&mut self, url: Url) -> Result<Step<Colour>> {
        let run = self.book.ledger().config.run_id.clone();
        let prior = self
            .book
            .index()
            .evidence
            .values()
            .filter(|(_, evidence)| {
                evidence.author() == self.peer_id && evidence.run() == run && evidence.url() == &url
            })
            .count();
        let fetched = match fetch::fetch(
            self.source,
            &url,
            &self.day,
            self.peer_id,
            &run,
            prior,
            forthebirds_store::now(),
        )
        .await
        {
            Ok(evidence) => evidence,
            Err(error) => {
                // Beside the tool result. A validate refusal never reaches here.
                self.fetch_failures.push(FetchFailure {
                    code: error.code,
                    host: url.host_str().unwrap_or("").to_owned(),
                });
                return continue_with(json!({"error": error.code}));
            }
        };
        let evidence_id = fetched.id().0.clone();
        let excerpt = fetched.excerpt().to_owned();
        self.book.save_evidence(fetched).await?;
        self.completed_fetches += 1;
        continue_with(json!({"evidence_id": evidence_id, "excerpt": excerpt}))
    }

    async fn argue(&self, operation: &str, action: Action) -> Result<Step<Colour>> {
        match self.write_action(operation, action).await {
            Ok(record) => continue_with(json!({"posted": record.id})),
            Err(error) if soft_write(error.code) => continue_with(json!({"error": error.code})),
            Err(error) => Err(error),
        }
    }

    async fn stance(&self, operation: &str, action: Action) -> Result<Step<Colour>> {
        match self.write_action(operation, action).await {
            Ok(record) => continue_with(json!({"declared": record.id})),
            Err(error) if soft_write(error.code) => continue_with(json!({"error": error.code})),
            Err(error) => Err(error),
        }
    }

    async fn vote(&self, operation: &str, action: Action, colour: Colour) -> Result<Step<Colour>> {
        self.write_action(operation, action).await?;
        Ok(Step::Finish(colour))
    }

    async fn write_action(&self, operation: &str, action: Action) -> Result<ActionRecord<Action>> {
        let encoded = serde_json::to_string(&action).expect("action serializes");
        let memory_tokens = self.model.tokenize(&encoded).await?.len() as u64;
        self.book.write(operation, action, memory_tokens).await
    }
}

/// Messages are the system prompt and this turn's JSON context. Operation ids
/// are the runtime's `{activation}-{turn}`.
#[allow(clippy::too_many_arguments)]
pub async fn activate<S: Source>(
    book: &Book,
    model: &Model,
    source: &S,
    peer_id: &str,
    day: Day,
    clock: DayClock,
    limits: Limits,
    activation: &str,
    fetch_failures: &mut Vec<FetchFailure>,
) -> Result<Colour> {
    let index = book.index();
    let gate = prompt::vote_gate(&index, peer_id, &book.ledger().config.run_id);
    let context = prompt::context(&day, &clock, peer_id, &index, FETCHES_PER_ACTIVATION, gate);
    let mut messages = vec![
        json!({"role": "system", "content": prompt::system_prompt()}),
        json!({"role": "user", "content": context.to_string()}),
    ];
    let mut host = PeerActivation {
        book,
        model,
        source,
        peer_id,
        day,
        clock,
        completed_fetches: 0,
        fetch_failures,
    };
    forthebirds_runtime::activation::run(
        model,
        &prompt::schema_for(&day, peer_id),
        limits,
        &mut host,
        &mut messages,
        activation,
    )
    .await
}

fn continue_with(value: Value) -> Result<Step<Colour>> {
    Ok(Step::Continue(value.to_string()))
}

fn soft_write(code: &str) -> bool {
    matches!(
        code,
        "refused" | "invalid_action" | "missing_reference" | "budget_exhausted" | "limit"
    )
}

fn own_evidence(book: &Book, peer_id: &str, run: &str) -> Vec<Evidence> {
    book.index()
        .evidence
        .values()
        .filter(|(_, evidence)| evidence.author() == peer_id && evidence.run() == run)
        .map(|(_, evidence)| evidence.clone())
        .collect()
}

/// Oldest argument first, then oldest own evidence. Returns false when the
/// context has nothing left that a prompt may drop.
fn drop_oldest(context: &mut Value) -> bool {
    take_first(context, "arguments") || take_first(context, "your_evidence")
}

fn take_first(context: &mut Value, key: &str) -> bool {
    let Some(items) = context.get_mut(key).and_then(Value::as_array_mut) else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    items.remove(0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::closing;
    use crate::config::TEMPERATURE;
    use crate::fetch::FetchError;
    use crate::record::Record;
    use axum::{Json, Router, extract::State, routing::post};
    use forthebirds_inference::InferenceConfig;
    use forthebirds_store::LedgerConfig;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn day() -> Day {
        "2026-10-26".parse().unwrap()
    }

    fn clock() -> DayClock {
        let start = day().open_ms();
        DayClock {
            real_start_ms: start,
            sim_start_ms: start,
            speed: 1,
        }
    }

    fn limits(max_steps: usize) -> Limits {
        Limits {
            max_steps,
            output_tokens: 64,
            continuation: false,
        }
    }

    fn config(path: &Path, peer: &str) -> LedgerConfig {
        LedgerConfig {
            database_path: path.to_path_buf(),
            database_cache_bytes: 1024 * 1024,
            run_id: "run-1".into(),
            peer_id: peer.into(),
            token_budget: Some(10_000),
            memory_budget: 1_000,
            memory_item_tokens: 100,
            max_documents: 100,
        }
    }

    fn wiki_body() -> Vec<u8> {
        br#"{"events":[{"year":1903,"text":"A fixed fact about the day"},{"year":1912,"text":"A second fixed fact"},{"year":1955,"text":"A third fixed fact"}]}"#.to_vec()
    }

    fn births_body() -> Vec<u8> {
        br#"{"births":[{"year":1903,"text":"A fixed birth on this day"}]}"#.to_vec()
    }

    struct FixedSource {
        calls: Arc<AtomicUsize>,
    }

    impl Source for FixedSource {
        fn get(
            &self,
            url: &Url,
        ) -> impl Future<Output = std::result::Result<Vec<u8>, FetchError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            let body = match crate::evidence::Allowlist::list(&url) {
                Some("events") => wiki_body(),
                Some("births") => births_body(),
                _ => Vec::new(),
            };
            async move {
                if body.is_empty() {
                    Err(FetchError {
                        code: "not_allowed",
                        message: format!("refused {url}"),
                    })
                } else {
                    Ok(body)
                }
            }
        }
    }

    /// Allowlisted births fail in the source; the events list still succeeds.
    /// `actions::validate` never sees this `not_allowed`: the URL is permitted.
    struct BirthsRefused {
        calls: Arc<AtomicUsize>,
    }

    impl Source for BirthsRefused {
        fn get(
            &self,
            url: &Url,
        ) -> impl Future<Output = std::result::Result<Vec<u8>, FetchError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            async move {
                if crate::evidence::Allowlist::list(&url) == Some("births") {
                    Err(FetchError {
                        code: "not_allowed",
                        message: format!("refused {url}"),
                    })
                } else if crate::evidence::Allowlist::list(&url) == Some("events") {
                    Ok(wiki_body())
                } else {
                    Err(FetchError {
                        code: "not_allowed",
                        message: format!("refused {url}"),
                    })
                }
            }
        }
    }

    fn failure_counts(failures: &[FetchFailure]) -> std::collections::BTreeMap<&str, u64> {
        let mut counts = std::collections::BTreeMap::new();
        for failure in failures {
            *counts.entry(failure.code).or_default() += 1;
        }
        counts
    }

    #[derive(Clone, Copy)]
    enum Script {
        FetchWiki,
        FetchBirths,
        FetchObservances,
        ArgueLastEvidence,
        ArgueUnknown,
        ArgueReply,
        ArgueUnknownReply,
        Invalid,
        VoteBlue,
        StanceBlue,
        StanceBlueAcceptTeal,
        StanceRed,
        StanceRefuseRedUncited,
        StanceRefuseRedCited,
    }

    #[derive(Clone)]
    struct Fake {
        requests: Arc<Mutex<Vec<(String, Value)>>>,
        script: Arc<Mutex<VecDeque<Script>>>,
        last_context: Arc<Mutex<Value>>,
        wiki_url: String,
    }

    async fn serve(fake: Fake) -> (String, tokio::task::JoinHandle<()>) {
        let router = Router::new()
            .route("/apply-template", post(apply_template))
            .route("/tokenize", post(tokenize))
            .route("/completion", post(completion))
            .with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (url, task)
    }

    async fn apply_template(State(fake): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
        if let Some(content) = body.pointer("/messages/1/content").and_then(Value::as_str)
            && let Ok(parsed) = serde_json::from_str::<Value>(content)
        {
            *fake.last_context.lock().unwrap() = parsed;
        }
        fake.requests
            .lock()
            .unwrap()
            .push(("/apply-template".into(), body));
        Json(json!({"prompt": "plumage-test-prompt"}))
    }

    async fn tokenize(State(fake): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
        fake.requests
            .lock()
            .unwrap()
            .push(("/tokenize".into(), body));
        Json(json!({"tokens": [1, 2, 3]}))
    }

    async fn completion(State(fake): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
        fake.requests
            .lock()
            .unwrap()
            .push(("/completion".into(), body));
        let step = fake.script.lock().unwrap().pop_front();
        let action = match step {
            Some(Script::FetchObservances) => {
                json!({"action": "fetch", "url": crate::observances::URL})
            }
            Some(Script::FetchWiki) => json!({"action": "fetch", "url": fake.wiki_url}),
            Some(Script::FetchBirths) => {
                json!({"action": "fetch", "url": fetch::sources(&day(), "p00")[1].to_string()})
            }
            Some(
                Script::StanceBlue
                | Script::StanceBlueAcceptTeal
                | Script::StanceRed
                | Script::StanceRefuseRedUncited
                | Script::StanceRefuseRedCited,
            ) => {
                let prefer = if matches!(step, Some(Script::StanceRed)) {
                    "red"
                } else {
                    "blue"
                };
                let accept = if matches!(step, Some(Script::StanceBlueAcceptTeal)) {
                    vec!["teal"]
                } else {
                    vec![]
                };
                let refuse = if matches!(
                    step,
                    Some(Script::StanceRefuseRedUncited | Script::StanceRefuseRedCited)
                ) {
                    vec!["red"]
                } else {
                    vec![]
                };
                let cites = if matches!(step, Some(Script::StanceRefuseRedCited)) {
                    let context = fake.last_context.lock().unwrap();
                    vec![
                        context["your_evidence"].as_array().unwrap().last().unwrap()["id"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    ]
                } else {
                    vec![]
                };
                json!({"action":"stance","prefer":[prefer],"accept":accept,"refuse":refuse,"cites":cites})
            }
            Some(Script::ArgueLastEvidence) => {
                let context = fake.last_context.lock().unwrap().clone();
                let id = context
                    .pointer("/your_evidence")
                    .and_then(Value::as_array)
                    .and_then(|items| items.last())
                    .and_then(|item| item["id"].as_str())
                    .unwrap_or("missing-evidence")
                    .to_owned();
                json!({"action": "argue", "text": "the day points to blue", "cites": [id]})
            }
            Some(Script::ArgueUnknown) => {
                json!({"action": "argue", "text": "citing nothing I fetched", "cites": ["unknown-evidence-id"]})
            }
            Some(Script::ArgueReply) => {
                let context = fake.last_context.lock().unwrap().clone();
                let id = context
                    .pointer("/arguments")
                    .and_then(Value::as_array)
                    .and_then(|items| items.last())
                    .and_then(|item| item["id"].as_str())
                    .unwrap_or("r:a:missing")
                    .to_owned();
                json!({"action": "argue", "text": "answering that", "cites": [], "reply_to": id})
            }
            Some(Script::ArgueUnknownReply) => {
                json!({"action": "argue", "text": "answering nobody", "cites": [], "reply_to": format!("r:a:{}", "ab".repeat(32))})
            }
            Some(Script::Invalid) => json!({"action": "name"}),
            Some(Script::VoteBlue) => json!({"action": "vote", "colour": "blue"}),
            None => json!({"action": "vote", "colour": "blue"}),
        };
        Json(json!({
            "content": action.to_string(),
            "truncated": false,
            "timings": {"predicted_n": 4}
        }))
    }

    fn model(url: &str) -> Model {
        Model::new(&InferenceConfig {
            model_url: url.to_owned(),
            embedding_url: None,
            context_tokens: 4096,
            output_tokens: 64,
            temperature: TEMPERATURE,
            render_tokens: None,
            inference_timeout_seconds: 30,
            request_timeout_seconds: 30,
            user_agent: "plumage-activation-test".into(),
        })
        .unwrap()
    }

    async fn open_book(dir: &Path, name: &str, peer: &str) -> Book {
        Book::open(config(&dir.join(name), peer)).await.unwrap()
    }

    fn fake(peer: &str, script: Vec<Script>) -> Fake {
        let wiki_url = fetch::sources(&day(), peer)[0].to_string();
        Fake {
            requests: Arc::new(Mutex::new(Vec::new())),
            script: Arc::new(Mutex::new(script.into())),
            last_context: Arc::new(Mutex::new(Value::Null)),
            wiki_url,
        }
    }

    fn tool_results(requests: &[(String, Value)]) -> Vec<String> {
        let Some((_, body)) = requests
            .iter()
            .rev()
            .find(|(path, _)| path == "/apply-template")
        else {
            return Vec::new();
        };
        body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|message| {
                message["content"]
                    .as_str()
                    .and_then(|content| content.strip_prefix("Tool result: "))
                    .map(str::to_owned)
            })
            .collect()
    }

    fn assert_schemas(requests: &[(String, Value)], peer: &str, may_vote: &[Option<&[Colour]>]) {
        let completions: Vec<_> = requests
            .iter()
            .filter(|(path, _)| path == "/completion")
            .collect();
        assert_eq!(completions.len(), may_vote.len());
        for ((_, body), allowed) in completions.into_iter().zip(may_vote) {
            assert_eq!(
                body["json_schema"],
                prompt::schema_allowing(&day(), peer, *allowed)
            );
        }
    }

    fn assert_context_matches_next_schema(requests: &[(String, Value)]) {
        let mut pending = None;
        let mut turns = 0;
        for (path, body) in requests {
            match path.as_str() {
                "/apply-template" => {
                    assert!(pending.is_none(), "each template must reach a completion");
                    assert_eq!(body["messages"][0]["content"], prompt::system_prompt());
                    let context: Value =
                        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap())
                            .unwrap();
                    pending = Some(context);
                }
                "/completion" => {
                    let context = pending.take().expect("a context before each completion");
                    let offers_vote = body["json_schema"]["oneOf"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|choice| choice["properties"]["action"]["const"] == "vote");
                    assert_eq!(context["may_vote"], offers_vote);
                    assert_eq!(
                        context["before_vote"].as_array().unwrap().is_empty(),
                        offers_vote
                    );
                    turns += 1;
                }
                _ => {}
            }
        }
        assert!(pending.is_none());
        assert_eq!(turns, 4);
    }

    #[test]
    fn drop_oldest_removes_arguments_then_own_evidence() {
        let mut context = json!({
            "arguments": [{"id": "a1"}, {"id": "a2"}],
            "your_evidence": [{"id": "e1"}, {"id": "e2"}],
            "may_vote": true,
        });
        for (arguments, evidence) in [
            (json!([{"id": "a2"}]), json!([{"id": "e1"}, {"id": "e2"}])),
            (json!([]), json!([{"id": "e1"}, {"id": "e2"}])),
            (json!([]), json!([{"id": "e2"}])),
            (json!([]), json!([])),
        ] {
            assert!(drop_oldest(&mut context));
            assert_eq!(context["arguments"], arguments);
            assert_eq!(context["your_evidence"], evidence);
            assert_eq!(context["may_vote"], true);
        }
        assert!(!drop_oldest(&mut context));
    }

    async fn settled(book: &Book) {
        let cache = book.ledger().cache.lock().await;
        assert_eq!(cache.pending_tokens, 0);
        assert!(cache.inference_charged > 0);
    }

    #[tokio::test]
    async fn scripted_fetch_argue_vote_settles_each_step() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::StanceBlueAcceptTeal,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let colour = activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-1",
            &mut Vec::new(),
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        let index = book.index();
        assert_eq!(index.evidence.len(), 1);
        assert_eq!(index.arguments.len(), 1);
        assert_eq!(index.stances.len(), 1);
        assert_eq!(
            index.stances["p00"].vote_choices(),
            [Colour::Blue, Colour::Teal]
        );
        assert_eq!(index.arguments[0].cites.len(), 1);
        assert_eq!(index.votes.len(), 1);
        assert_eq!(index.votes[0].colour, Colour::Blue);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        settled(&book).await;
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(
            serde_json::from_str::<Value>(&results[1]).unwrap(),
            json!({"declared":index.stances["p00"].id})
        );
        assert_schemas(
            &requests,
            "p00",
            &[None, None, None, Some(&[Colour::Blue, Colour::Teal])],
        );

        let server = fake("p00", vec![Script::VoteBlue]);
        let (url, task) = serve(server.clone()).await;
        let next_model = self::model(&url);
        let colour = activate(
            &book,
            &next_model,
            &source,
            "p00",
            day(),
            clock(),
            limits(1),
            "act-2",
            &mut Vec::new(),
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        assert_eq!(book.index().votes.len(), 2);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        assert_schemas(
            &server.requests.lock().unwrap(),
            "p00",
            &[Some(&[Colour::Blue, Colour::Teal])],
        );
    }

    #[tokio::test]
    async fn fourth_fetch_is_budget_exhausted_and_three_evidence_remain() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::FetchWiki,
                Script::FetchWiki,
                Script::FetchWiki,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let mut failures = Vec::new();
        activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-budget",
            &mut failures,
        )
        .await
        .unwrap();
        task.abort();
        assert!(failures.is_empty());
        assert_eq!(source.calls.load(Ordering::SeqCst), 3);
        assert_eq!(book.index().evidence.len(), 3);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[3], r#"{"refused":"BudgetExhausted"}"#);
        assert_schemas(
            &requests,
            "p00",
            &[None, None, None, None, None, None, Some(&[Colour::Blue])],
        );
    }

    #[tokio::test]
    async fn invalid_action_and_unknown_citation_continue_and_the_vote_lands() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::Invalid,
                Script::ArgueUnknown,
                Script::VoteBlue,
                Script::FetchWiki,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let colour = activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-refuse",
            &mut Vec::new(),
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        assert_eq!(book.index().votes.len(), 1);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[0], r#"{"error":"invalid_action"}"#);
        assert_eq!(results[1], r#"{"refused":"UnknownCitation"}"#);
        let result: Value = serde_json::from_str(&results[2]).unwrap();
        assert_eq!(
            result,
            json!({"refused": "VoteBeforeReading", "reason": "fetch one source and post one argument first; a vote needs both in this run"})
        );
        assert_schemas(
            &requests,
            "p00",
            &[None, None, None, None, None, None, Some(&[Colour::Blue])],
        );
    }

    #[tokio::test]
    async fn reply_to_a_peers_argument_posts_and_an_unknown_reply_continues_to_vote() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_book(dir.path(), "p01.redb", "p01").await;
        let evidence = Evidence::new(
            "p01".into(),
            "run-1".into(),
            fetch::sources(&day(), "p00")[0].clone(),
            day().open_ms(),
            "a fact",
        );
        let evidence_id = evidence.id().clone();
        author.save_evidence(evidence).await.unwrap();
        let target = author
            .write(
                "argue-1",
                Action::Argue {
                    text: "the day is blue".into(),
                    cites: vec![evidence_id],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        let page = author.ledger().page(None, 16).await.unwrap();
        let mut evidence_records = Vec::new();
        let mut action_records = Vec::new();
        for record in page.records {
            match record {
                Record::Evidence(record) => evidence_records.push(Record::Evidence(record)),
                Record::Action(record) => action_records.push(Record::Action(record)),
            }
        }
        let reader = open_book(dir.path(), "p00.redb", "p00").await;
        reader
            .ledger()
            .import(&reader, evidence_records)
            .await
            .unwrap();
        reader
            .ledger()
            .import(&reader, action_records)
            .await
            .unwrap();

        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::StanceBlue,
                Script::ArgueReply,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let reply_model = model(&url);
        let colour = activate(
            &reader,
            &reply_model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-reply",
            &mut Vec::new(),
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        let posted = reader
            .index()
            .arguments
            .into_iter()
            .find(|argument| argument.author == "p00")
            .unwrap();
        assert_eq!(posted.text, "answering that");
        assert_eq!(posted.reply_to.as_deref(), Some(target.id.as_str()));

        let fresh = tempfile::tempdir().unwrap();
        let book = open_book(fresh.path(), "p00.redb", "p00").await;
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::ArgueUnknownReply,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let unknown_model = model(&url);
        let colour = activate(
            &book,
            &unknown_model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-unknown-reply",
            &mut Vec::new(),
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        assert_eq!(book.index().arguments.len(), 1);
        assert_eq!(book.index().votes.len(), 1);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[1], r#"{"refused":"UnknownReply"}"#);
    }

    #[tokio::test]
    async fn no_vote_within_max_steps_is_tool_limit_and_records_remain() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake("p00", vec![Script::FetchWiki, Script::ArgueLastEvidence]);
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let error = activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(2),
            "act-limit",
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        task.abort();
        assert_eq!(error.code, "tool_limit");
        let index = book.index();
        assert_eq!(index.evidence.len(), 1);
        assert_eq!(index.arguments.len(), 1);
        settled(&book).await;
    }

    #[tokio::test]
    async fn system_prompt_is_identical_and_another_argument_is_only_in_the_context() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_book(dir.path(), "p01.redb", "p01").await;
        let evidence = Evidence::new(
            "p01".into(),
            "run-1".into(),
            fetch::sources(&day(), "p00")[0].clone(),
            day().open_ms(),
            "EXCERPT-ONLY-P01-SHOULD-NOT-LEAK",
        );
        let evidence_id = evidence.id().clone();
        author.save_evidence(evidence).await.unwrap();
        author
            .write(
                "argue-1",
                Action::Argue {
                    text: "ARGUMENT-P01-VISIBLE".into(),
                    cites: vec![evidence_id],
                    reply_to: None,
                },
                1,
            )
            .await
            .unwrap();
        let page = author.ledger().page(None, 16).await.unwrap();
        let mut evidence_records = Vec::new();
        let mut action_records = Vec::new();
        for record in page.records {
            match record {
                Record::Evidence(record) => evidence_records.push(Record::Evidence(record)),
                Record::Action(record) => action_records.push(Record::Action(record)),
            }
        }
        let reader = open_book(dir.path(), "p00.redb", "p00").await;
        reader
            .ledger()
            .import(&reader, evidence_records)
            .await
            .unwrap();
        reader
            .ledger()
            .import(&reader, action_records)
            .await
            .unwrap();

        author
            .write(
                "stance-0",
                Action::Stance {
                    prefer: vec![Colour::Blue],
                    accept: vec![],
                    refuse: vec![],
                    cites: vec![],
                },
                1,
            )
            .await
            .unwrap();
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut prompts = Vec::new();
        for (peer, book) in [("p00", &reader), ("p01", &author)] {
            let server = fake(
                peer,
                vec![
                    Script::FetchWiki,
                    Script::StanceBlue,
                    Script::ArgueLastEvidence,
                    Script::VoteBlue,
                ],
            );
            let (url, task) = serve(server.clone()).await;
            let model = model(&url);
            activate(
                book,
                &model,
                &source,
                peer,
                day(),
                clock(),
                limits(4),
                "act-prompt",
                &mut Vec::new(),
            )
            .await
            .unwrap();
            task.abort();
            let requests = server.requests.lock().unwrap().clone();
            let (_, body) = requests
                .iter()
                .find(|(path, _)| path == "/apply-template")
                .unwrap();
            let messages = body["messages"].as_array().unwrap();
            prompts.push((
                messages[0]["content"].as_str().unwrap().to_owned(),
                serde_json::from_str::<Value>(messages[1]["content"].as_str().unwrap()).unwrap(),
                messages[1]["content"].as_str().unwrap().to_owned(),
            ));
            assert_context_matches_next_schema(&requests);
            assert_schemas(
                &requests,
                peer,
                if peer == "p00" {
                    &[None, None, None, Some(&[Colour::Blue])]
                } else {
                    &[
                        Some(&[Colour::Blue]),
                        Some(&[Colour::Blue]),
                        Some(&[Colour::Blue]),
                        Some(&[Colour::Blue]),
                    ]
                },
            );
        }
        assert_eq!(prompts[0].0, prompt::system_prompt());
        assert_eq!(prompts[0].0, prompts[1].0);
        assert!(prompts[0].0.contains(closing::RULE));
        assert_eq!(prompts[0].1["you"], "p00");
        assert_eq!(prompts[1].1["you"], "p01");
        assert_eq!(prompts[0].1["date"], "2026-10-26");
        for ((_, context, _), (peer, edition)) in prompts.iter().zip([("p00", "en"), ("p01", "fr")])
        {
            assert_eq!(context["edition"], edition);
            assert_eq!(
                context["sources"],
                json!(
                    fetch::sources(&day(), peer)
                        .into_iter()
                        .map(|url| url.to_string())
                        .collect::<Vec<_>>()
                )
            );
        }
        assert!(prompts[0].2.contains("ARGUMENT-P01-VISIBLE"));
        assert!(!prompts[0].0.contains("ARGUMENT-P01-VISIBLE"));
        assert!(!prompts[0].2.contains("EXCERPT-ONLY-P01-SHOULD-NOT-LEAK"));
        assert!(!prompts[0].0.contains("EXCERPT-ONLY-P01-SHOULD-NOT-LEAK"));
    }

    #[tokio::test]
    async fn refused_allowlisted_fetch_is_counted_and_the_vote_still_lands() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = BirthsRefused {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchBirths,
                Script::FetchWiki,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let mut failures = Vec::new();
        let colour = activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-fetch-fail",
            &mut failures,
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        let mut expected = std::collections::BTreeMap::new();
        expected.insert("not_allowed", 1);
        assert_eq!(failure_counts(&failures), expected);
        assert_eq!(failures[0].host, crate::evidence::WIKIMEDIA_HOST);
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        assert_eq!(book.index().evidence.len(), 1);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[0], r#"{"error":"not_allowed"}"#);
    }

    #[tokio::test]
    async fn observance_fetch_is_evidence_without_a_source_call_and_costs_a_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchObservances,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let day: Day = "2026-02-02".parse().unwrap();
        let clock = DayClock {
            real_start_ms: day.open_ms(),
            sim_start_ms: day.open_ms(),
            speed: 1,
        };
        let mut failures = Vec::new();
        let colour = activate(
            &book,
            &model,
            &source,
            "p00",
            day,
            clock,
            limits(8),
            "act-un",
            &mut failures,
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        assert!(failures.is_empty());
        let index = book.index();
        assert_eq!(index.evidence.len(), 1);
        let (_, evidence) = index.evidence.values().next().unwrap();
        assert_eq!(evidence.url().as_str(), crate::observances::URL);
        assert_eq!(evidence.excerpt(), "World Wetlands Day (A/RES/75/317)");
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert!(results[0].starts_with(r#"{"evidence_id":"#));
        assert!(results[0].contains(evidence.excerpt()));
        let contexts: Vec<Value> = requests
            .iter()
            .filter(|(path, _)| path == "/apply-template")
            .map(|(_, body)| {
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
            })
            .collect();
        assert_eq!(contexts[0]["sources"].as_array().unwrap().len(), 6);
        assert_eq!(contexts[0]["sources"][5], crate::observances::URL);
        assert_eq!(contexts[0]["fetches_left"], 3);
        assert_eq!(contexts[1]["fetches_left"], 2);
        let completions: Vec<_> = requests
            .iter()
            .filter(|(path, _)| path == "/completion")
            .collect();
        assert_eq!(completions.len(), 4);
        for ((_, body), may_vote) in
            completions
                .iter()
                .zip([None, None, None, Some(&[Colour::Blue][..])])
        {
            assert_eq!(
                body["json_schema"],
                prompt::schema_allowing(&day, "p00", may_vote)
            );
        }
        settled(&book).await;
    }

    #[tokio::test]
    async fn unnamed_calendar_day_reports_failure_then_the_vote_lands() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchObservances,
                Script::FetchWiki,
                Script::StanceBlue,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        let model = model(&url);
        let mut failures = Vec::new();
        let colour = activate(
            &book,
            &model,
            &source,
            "p00",
            day(),
            clock(),
            limits(8),
            "act-un-empty",
            &mut failures,
        )
        .await
        .unwrap();
        task.abort();
        assert_eq!(colour, Colour::Blue);
        assert_eq!(
            failures,
            vec![FetchFailure {
                code: "source_invalid",
                host: crate::observances::HOST.into()
            }]
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        assert_eq!(book.index().evidence.len(), 1);
        let requests = server.requests.lock().unwrap().clone();
        assert_eq!(tool_results(&requests)[0], r#"{"error":"source_invalid"}"#);
    }

    #[tokio::test]
    async fn a_refused_vote_can_redeclare_a_stance_and_vote_with_the_new_grammar() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::StanceRed,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
                Script::StanceBlue,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        assert_eq!(
            activate(
                &book,
                &model(&url),
                &source,
                "p00",
                day(),
                clock(),
                limits(8),
                "redeclare",
                &mut Vec::new()
            )
            .await
            .unwrap(),
            Colour::Blue
        );
        task.abort();
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(
            serde_json::from_str::<Value>(&results[3]).unwrap(),
            json!({"refused":"VoteOutsideStance","reason":"vote a colour in your current prefer or accept lists, or declare a new stance first"})
        );
        assert_schemas(
            &requests,
            "p00",
            &[
                None,
                None,
                None,
                Some(&[Colour::Red]),
                Some(&[Colour::Red]),
                Some(&[Colour::Blue]),
            ],
        );
        assert_eq!(book.index().stances["p00"].prefer, [Colour::Blue]);
        assert_eq!(book.ledger().page(None,32).await.unwrap().records.iter().filter(|r| matches!(r,crate::record::Record::Action(a) if matches!(a.action,Action::Stance { .. }))).count(),2);
    }

    #[tokio::test]
    async fn missing_stance_and_uncited_refusal_continue_until_a_valid_stance_and_vote() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(
            "p00",
            vec![
                Script::FetchWiki,
                Script::ArgueLastEvidence,
                Script::VoteBlue,
                Script::StanceRefuseRedUncited,
                Script::StanceRefuseRedCited,
                Script::VoteBlue,
            ],
        );
        let (url, task) = serve(server.clone()).await;
        assert_eq!(
            activate(
                &book,
                &model(&url),
                &source,
                "p00",
                day(),
                clock(),
                limits(8),
                "missing-stance",
                &mut Vec::new()
            )
            .await
            .unwrap(),
            Colour::Blue
        );
        task.abort();
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(
            serde_json::from_str::<Value>(&results[2]).unwrap(),
            json!({"refused":"VoteBeforeStance","reason":"declare a stance first; a vote needs one fetch, a stance and one argument in this run"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&results[3]).unwrap(),
            json!({"refused":"StanceRefusalUncited","reason":"a refusal cites at least one evidence id you fetched this run; cite one or leave refuse empty"})
        );
        assert_schemas(
            &requests,
            "p00",
            &[None, None, None, None, None, Some(&[Colour::Blue])],
        );
        let index = book.index();
        assert_eq!(index.stances["p00"].cites.len(), 1);
        assert_eq!(index.votes.len(), 1);
    }
}
