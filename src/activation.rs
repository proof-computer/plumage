//! One bounded activation: fetch, argue, vote, then stop.

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
        *context = prompt::context(&self.day, &self.clock, self.peer_id, &index, fetches_left);
        Ok(None)
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
        let (arguments, other_records) = self.book.index().reply_records();
        let check = actions::Context {
            author: self.peer_id,
            run: &run,
            evidence: &own,
            fetches_this_activation: self.completed_fetches,
            arguments: &arguments,
            other_records: &other_records,
        };
        if let Err(refusal) = actions::validate(&action, &check) {
            return continue_with(json!({"refused": format!("{refusal:?}")}));
        }
        match action {
            Action::Fetch { url } => self.fetch(url).await,
            Action::Argue { .. } => self.argue(operation, action).await,
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
    let context = prompt::context(&day, &clock, peer_id, &index, FETCHES_PER_ACTIVATION);
    let mut messages = vec![
        json!({"role": "system", "content": prompt::SYSTEM_PROMPT}),
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
        &prompt::schema_for(&day),
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

    fn sunrise_body() -> Vec<u8> {
        br#"{"status":"OK","results":{"sunrise":"2026-10-26T06:40:00+00:00","sunset":"2026-10-26T16:35:00+00:00"}}"#.to_vec()
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
            let body = match url.host_str() {
                Some(crate::evidence::WIKIMEDIA_HOST) => wiki_body(),
                Some(crate::evidence::SUNRISE_HOST) => sunrise_body(),
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

    /// Allowlisted sunrise fails in the source; the wiki host still succeeds.
    /// `actions::validate` never sees this `not_allowed`: the URL is permitted.
    struct SunriseRefused {
        calls: Arc<AtomicUsize>,
    }

    impl Source for SunriseRefused {
        fn get(
            &self,
            url: &Url,
        ) -> impl Future<Output = std::result::Result<Vec<u8>, FetchError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            async move {
                if url.host_str() == Some(crate::evidence::SUNRISE_HOST) {
                    Err(FetchError {
                        code: "not_allowed",
                        message: format!("refused {url}"),
                    })
                } else if url.host_str() == Some(crate::evidence::WIKIMEDIA_HOST) {
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
        FetchSunrise,
        ArgueLastEvidence,
        ArgueUnknown,
        ArgueReply,
        ArgueUnknownReply,
        Invalid,
        VoteBlue,
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
            Some(Script::FetchWiki) => json!({"action": "fetch", "url": fake.wiki_url}),
            Some(Script::FetchSunrise) => {
                json!({"action": "fetch", "url": fetch::sources(&day())[1].to_string()})
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

    fn fake(script: Vec<Script>) -> Fake {
        let wiki_url = fetch::sources(&day())[0].to_string();
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

    fn assert_narrowed_schema(requests: &[(String, Value)]) {
        let expected = prompt::schema_for(&day());
        let completions: Vec<_> = requests
            .iter()
            .filter(|(path, _)| path == "/completion")
            .collect();
        assert!(!completions.is_empty());
        for (_, body) in completions {
            assert_eq!(body["json_schema"], expected);
        }
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
        let server = fake(vec![
            Script::FetchWiki,
            Script::ArgueLastEvidence,
            Script::VoteBlue,
        ]);
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
        assert_eq!(index.arguments[0].cites.len(), 1);
        assert_eq!(index.votes.len(), 1);
        assert_eq!(index.votes[0].colour, Colour::Blue);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        settled(&book).await;
        let requests = server.requests.lock().unwrap().clone();
        assert_narrowed_schema(&requests);
    }

    #[tokio::test]
    async fn fourth_fetch_is_budget_exhausted_and_three_evidence_remain() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(vec![
            Script::FetchWiki,
            Script::FetchWiki,
            Script::FetchWiki,
            Script::FetchWiki,
            Script::VoteBlue,
        ]);
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
        assert_narrowed_schema(&requests);
    }

    #[tokio::test]
    async fn invalid_action_and_unknown_citation_continue_and_the_vote_lands() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(vec![
            Script::Invalid,
            Script::ArgueUnknown,
            Script::VoteBlue,
        ]);
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
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[0], r#"{"error":"invalid_action"}"#);
        assert_eq!(results[1], r#"{"refused":"UnknownCitation"}"#);
    }

    #[tokio::test]
    async fn reply_to_a_peers_argument_posts_and_an_unknown_reply_continues_to_vote() {
        let dir = tempfile::tempdir().unwrap();
        let author = open_book(dir.path(), "p01.redb", "p01").await;
        let evidence = Evidence::new(
            "p01".into(),
            "run-1".into(),
            fetch::sources(&day())[0].clone(),
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
        let server = fake(vec![Script::ArgueReply, Script::VoteBlue]);
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
        let server = fake(vec![Script::ArgueUnknownReply, Script::VoteBlue]);
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
        assert!(book.index().arguments.is_empty());
        assert_eq!(book.index().votes.len(), 1);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[0], r#"{"refused":"UnknownReply"}"#);
    }

    #[tokio::test]
    async fn no_vote_within_max_steps_is_tool_limit_and_records_remain() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(vec![Script::FetchWiki, Script::ArgueLastEvidence]);
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
            fetch::sources(&day())[0].clone(),
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

        let source = FixedSource {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut prompts = Vec::new();
        for (peer, book) in [("p00", &reader), ("p01", &author)] {
            let server = fake(vec![Script::VoteBlue]);
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
            assert_narrowed_schema(&requests);
        }
        assert_eq!(prompts[0].0, prompts[1].0);
        assert!(prompts[0].0.contains(closing::RULE));
        assert_eq!(prompts[0].1["you"], "p00");
        assert_eq!(prompts[1].1["you"], "p01");
        assert_eq!(prompts[0].1["date"], "2026-10-26");
        assert_eq!(prompts[0].1["sources"].as_array().unwrap().len(), 2);
        let sources = fetch::sources(&day());
        assert_eq!(prompts[0].1["sources"][0], sources[0].as_str());
        assert_eq!(prompts[0].1["sources"][1], sources[1].as_str());
        assert!(prompts[0].2.contains("ARGUMENT-P01-VISIBLE"));
        assert!(!prompts[0].0.contains("ARGUMENT-P01-VISIBLE"));
        assert!(!prompts[0].2.contains("EXCERPT-ONLY-P01-SHOULD-NOT-LEAK"));
        assert!(!prompts[0].0.contains("EXCERPT-ONLY-P01-SHOULD-NOT-LEAK"));
    }

    #[tokio::test]
    async fn refused_allowlisted_fetch_is_counted_and_the_vote_still_lands() {
        let dir = tempfile::tempdir().unwrap();
        let book = open_book(dir.path(), "p00.redb", "p00").await;
        let source = SunriseRefused {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let server = fake(vec![
            Script::FetchSunrise,
            Script::FetchWiki,
            Script::VoteBlue,
        ]);
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
        assert_eq!(failures[0].host, crate::evidence::SUNRISE_HOST);
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        assert_eq!(book.index().evidence.len(), 1);
        let requests = server.requests.lock().unwrap().clone();
        let results = tool_results(&requests);
        assert_eq!(results[0], r#"{"error":"not_allowed"}"#);
    }
}
