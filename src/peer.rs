//! One peer process: a ledger per day, pinned sync, activations, and the site.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use forthebirds_inference::InferenceConfig;
use forthebirds_inference::model::Model;
use forthebirds_names::{bird_name, collision_ranks, numbered};
use forthebirds_net::{PeerTransport, StaticPeers, TransportConfig, TransportState};
use forthebirds_runtime::activation::Limits;
use forthebirds_store::LedgerConfig;
use forthebirds_sync::{Replicator, SyncConfig};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::activation;
use crate::book::{Argument, Book, DayIndex};
use crate::clock::DayClock;
use crate::closing::{self, Decision, Vote};
use crate::colour::Colour;
use crate::config::{DATABASE_CACHE_BYTES, MAX_DOCUMENTS, PeerConfig};
use crate::day::Day;
use crate::error::{Error, Result};
use crate::evidence::Evidence;
use crate::fetch::{self, Source};
use crate::site::{
    self, Activations, ArchivePage, ArgumentView, Chrome, DayPage, MAX_STATUS_BYTES, Paint, Phase,
    StatusInput, render_archive, render_day, render_flock, render_front, render_how, render_status,
};

/// Run from `config.clock.day()` through `last_day`. `shutdown` returns `Ok`.
pub async fn run<S: Source + 'static>(
    config: PeerConfig,
    source: S,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    config.validate()?;
    let mut state = RunState {
        activations: Activations::default(),
        started_real_ms: forthebirds_store::now(),
    };
    tokio::fs::create_dir_all(&config.data_dir)
        .await
        .map_err(storage)?;
    tokio::fs::create_dir_all(config.site_dir.join("days"))
        .await
        .map_err(storage)?;
    if let Some(parent) = config.advertisement_file.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await.map_err(storage)?;
    }
    write_assets(&config.site_dir).await?;
    let mut day = config.clock.day();
    let previous_day = day.previous();
    let phase = if config.clock.now_ms() >= day.close_ms() {
        Phase::SyncOnly
    } else {
        Phase::Voting
    };
    write_status(
        &config,
        day,
        phase,
        None,
        &state,
        (
            previous_day,
            read_paint(&config.site_dir, previous_day).await?,
        ),
    )
    .await?;
    let model = Model::new(&InferenceConfig {
        model_url: config.model_url.clone(),
        embedding_url: None,
        context_tokens: config.context_tokens,
        output_tokens: config.output_tokens,
        render_tokens: None,
        inference_timeout_seconds: config.inference_timeout_seconds,
        request_timeout_seconds: config.request_timeout_seconds,
        user_agent: fetch::USER_AGENT.to_owned(),
    })?;
    let names = display_names(&config.peer_id, std::iter::empty());
    write_how(
        &config,
        &names,
        read_paint(&config.site_dir, previous_day).await?,
    )
    .await?;
    loop {
        if halted(&shutdown) {
            return Ok(());
        }
        if let Some(last) = config.last_day
            && day > last
        {
            return Ok(());
        }
        let stop = run_day(&config, &model, &source, day, &mut shutdown, &mut state).await?;
        if stop || config.last_day == Some(day) {
            return Ok(());
        }
        day = day.next();
    }
}

/// `true` when shutdown asked the loop to stop.
async fn run_day<S: Source>(
    config: &PeerConfig,
    model: &Model,
    source: &S,
    day: Day,
    shutdown: &mut watch::Receiver<bool>,
    state: &mut RunState,
) -> Result<bool> {
    let book = Book::open(ledger_config(config, day)).await?;
    let replicator = Replicator::new(
        book.clone(),
        SyncConfig {
            run_id: day.to_string(),
            peer_id: config.peer_id.clone(),
            token: config.token.clone(),
            request_timeout: Duration::from_secs(config.request_timeout_seconds),
        },
    )?;
    let peers = StaticPeers::new(config.peers.clone())?;
    let transport = PeerTransport::start(
        &TransportConfig {
            listen: config.listen,
            advertisement_file: Some(config.advertisement_file.clone()),
            service: "plumage".to_owned(),
            application: "plumage".to_owned(),
        },
        &TransportState::default(),
        replicator.router(),
    )
    .await?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let every = Duration::from_millis(config.sync_interval_ms);
    let syncing = replicator.clone();
    let sync_task = tokio::spawn(async move {
        syncing.run(every, move || peers.read(), stop_rx).await;
    });
    let mut session = DaySession {
        stop_tx: Some(stop_tx),
        sync_task: Some(sync_task),
        transport: Some(transport),
    };

    let limits = Limits {
        max_steps: usize::try_from(config.max_steps).unwrap_or(usize::MAX),
        output_tokens: config.output_tokens,
        continuation: false,
    };
    let mut last_start = None;
    loop {
        if halted(shutdown) {
            session.finish().await?;
            return Ok(true);
        }
        let now = config.clock.now_ms();
        if now >= day.close_ms() {
            let previous = day.previous();
            let paint = match read_paint(&config.site_dir, previous).await {
                Ok(paint) => paint,
                Err(error) => {
                    session.finish().await?;
                    return Err(error);
                }
            };
            let index = book.index();
            if let Err(error) = write_status(
                config,
                day,
                Phase::SyncOnly,
                Some(&index),
                state,
                (previous, paint),
            )
            .await
            {
                session.finish().await?;
                return Err(error);
            }
            break;
        }
        let due = last_start.is_none_or(|started: u64| {
            now.saturating_sub(started) >= config.activation_interval_ms
        });
        if !due {
            let wake = last_start
                .unwrap_or(now)
                .saturating_add(config.activation_interval_ms)
                .min(day.close_ms());
            if wait_until(&config.clock, wake, shutdown).await {
                session.finish().await?;
                return Ok(true);
            }
            continue;
        }
        last_start = Some(now);
        let id = format!("a{now}");
        match activation::activate(
            &book,
            model,
            source,
            &config.peer_id,
            day,
            config.clock,
            limits,
            &id,
        )
        .await
        {
            Ok(colour) => {
                state
                    .activations
                    .record_vote(colour, forthebirds_store::now());
                println!(
                    "plumage activation day={day} peer={} id={id} outcome=voted:{}",
                    config.peer_id,
                    colour.name()
                );
            }
            Err(error) => {
                state
                    .activations
                    .record_error(error.code, forthebirds_store::now());
                println!(
                    "plumage activation day={day} peer={} id={id} outcome=error:{}",
                    config.peer_id, error.code
                );
            }
        }
        if let Err(error) = write_live(config, &book, day).await {
            session.finish().await?;
            return Err(error);
        }
        let previous = day.previous();
        let paint = match read_paint(&config.site_dir, previous).await {
            Ok(paint) => paint,
            Err(error) => {
                session.finish().await?;
                return Err(error);
            }
        };
        let index = book.index();
        if let Err(error) = write_status(
            config,
            day,
            Phase::Voting,
            Some(&index),
            state,
            (previous, paint),
        )
        .await
        {
            session.finish().await?;
            return Err(error);
        }
    }
    if wait_until(&config.clock, day.next().open_ms(), shutdown).await {
        session.finish().await?;
        return Ok(true);
    }
    let published = publish_close(config, &book, day, state).await;
    let finished = session.finish().await;
    published?;
    finished?;
    drop(replicator);
    drop(book);
    Ok(false)
}

struct DaySession {
    stop_tx: Option<watch::Sender<bool>>,
    sync_task: Option<JoinHandle<()>>,
    transport: Option<PeerTransport>,
}

impl DaySession {
    async fn finish(&mut self) -> Result<()> {
        if let Some(stop) = self.stop_tx.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.sync_task.take() {
            let _ = task.await;
        }
        if let Some(transport) = self.transport.take() {
            transport.stop().await?;
        }
        Ok(())
    }
}

fn ledger_config(config: &PeerConfig, day: Day) -> LedgerConfig {
    LedgerConfig {
        database_path: config.data_dir.join(format!("{day}.redb")),
        database_cache_bytes: DATABASE_CACHE_BYTES,
        run_id: day.to_string(),
        peer_id: config.peer_id.clone(),
        token_budget: config.token_budget,
        memory_budget: config.memory_budget,
        memory_item_tokens: config.memory_item_tokens,
        max_documents: MAX_DOCUMENTS,
    }
}

async fn write_live(config: &PeerConfig, book: &Book, day: Day) -> Result<()> {
    let index = book.index();
    let painted_by = read_paint(&config.site_dir, day.previous()).await?;
    let now = config.clock.now_ms();
    let frame = frame_from(&index, &config.clock);
    let names = display_names(&config.peer_id, authors(&index));
    let chrome = chrome(&config.peer_id, painted_by, now, now, &names);
    let page = DayPage {
        day,
        votes: &frame.votes,
        arguments: &frame.arguments,
        evidence: &frame.evidence,
        decision: None,
        chrome,
    };
    write_atomic(
        &config.site_dir.join("index.html"),
        render_front(&page).as_bytes(),
    )
    .await?;
    write_atomic(
        &config.site_dir.join("flock.html"),
        render_flock(&page).as_bytes(),
    )
    .await?;
    write_atomic(
        &config.site_dir.join(format!("days/{day}.html")),
        render_day(&page).as_bytes(),
    )
    .await?;
    Ok(())
}

async fn publish_close(config: &PeerConfig, book: &Book, day: Day, state: &RunState) -> Result<()> {
    let index = book.index();
    let frame = frame_from(&index, &config.clock);
    let open = day.open_ms();
    let close = day.close_ms();
    let live = closing::live_voters(&frame.votes, open, close);
    let decision = closing::decide(&frame.votes, &live, open, close);
    let now = config.clock.now_ms();
    let names = display_names(&config.peer_id, authors(&index));
    let painted = paint_of(&decision);
    let document = DayDocument {
        day,
        decision: decision_json(&decision),
        live_voters: live.clone(),
        rule: closing::RULE,
        live_rule: closing::LIVE_RULE,
        records: records_json(&frame),
    };
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| Error::new("storage", error.to_string()))?;
    let mut published = bytes;
    published.push(b'\n');
    write_atomic(
        &config.site_dir.join(format!("days/{day}.json")),
        &published,
    )
    .await?;

    let closed = DayPage {
        day,
        votes: &frame.votes,
        arguments: &frame.arguments,
        evidence: &frame.evidence,
        decision: Some(decision),
        chrome: chrome(&config.peer_id, painted, close, now, &names),
    };
    write_atomic(
        &config.site_dir.join(format!("days/{day}.html")),
        render_day(&closed).as_bytes(),
    )
    .await?;

    let archive = read_archive(&config.site_dir).await?;
    let archive_names = display_names(&config.peer_id, std::iter::empty());
    let archive_chrome = chrome(&config.peer_id, painted, now, now, &archive_names);
    write_atomic(
        &config.site_dir.join("days/index.html"),
        render_archive(&ArchivePage {
            days: &archive,
            chrome: archive_chrome,
        })
        .as_bytes(),
    )
    .await?;

    let next = day.next();
    let today_names = display_names(&config.peer_id, std::iter::empty());
    let today = DayPage {
        day: next,
        votes: &[],
        arguments: &[],
        evidence: &[],
        decision: None,
        chrome: chrome(&config.peer_id, painted, now, now, &today_names),
    };
    write_atomic(
        &config.site_dir.join("index.html"),
        render_front(&today).as_bytes(),
    )
    .await?;
    write_atomic(
        &config.site_dir.join("flock.html"),
        render_flock(&today).as_bytes(),
    )
    .await?;
    write_how(config, &today_names, painted).await?;

    let label = match decision {
        Decision::Decided { colour, .. } => colour.name(),
        Decision::Undecided => "undecided",
    };
    let records = index.votes.len() + index.arguments.len() + index.evidence.len();
    println!(
        "plumage day day={day} peer={} decision={label} live={} records={records}",
        config.peer_id,
        live.len()
    );
    write_status(
        config,
        day,
        Phase::Closed,
        Some(&index),
        state,
        (day, painted),
    )
    .await
}

struct Frame {
    votes: Vec<Vote>,
    arguments: Vec<ArgumentView>,
    evidence: Vec<Evidence>,
}

fn frame_from(index: &DayIndex, clock: &DayClock) -> Frame {
    let votes = index
        .votes
        .iter()
        .map(|vote| Vote {
            author: vote.author.clone(),
            colour: vote.colour,
            at_ms: clock.sim(vote.at_ms),
        })
        .collect();
    let arguments = index
        .arguments
        .iter()
        .map(|argument| ArgumentView {
            id: argument.id.clone(),
            author: argument.author.clone(),
            at_ms: clock.sim(argument.at_ms),
            text: argument.text.clone(),
            cites: cited(index, argument),
        })
        .collect();
    let evidence = index
        .evidence
        .values()
        .map(|(_, evidence)| evidence.clone())
        .collect();
    Frame {
        votes,
        arguments,
        evidence,
    }
}

fn cited(index: &DayIndex, argument: &Argument) -> Vec<Evidence> {
    argument
        .cites
        .iter()
        .filter_map(|id| {
            index
                .evidence
                .get(&id.0)
                .map(|(_, evidence)| evidence.clone())
        })
        .collect()
}

fn authors(index: &DayIndex) -> impl Iterator<Item = &str> {
    index
        .votes
        .iter()
        .map(|vote| vote.author.as_str())
        .chain(
            index
                .arguments
                .iter()
                .map(|argument| argument.author.as_str()),
        )
        .chain(
            index
                .evidence
                .values()
                .map(|(_, evidence)| evidence.author()),
        )
}

fn display_names<'a>(
    peer_id: &str,
    authors: impl Iterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    let mut ids = BTreeSet::new();
    ids.insert(peer_id.to_owned());
    for author in authors {
        ids.insert(author.to_owned());
    }
    let mut by_base: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in &ids {
        by_base.entry(bird_name(id)).or_default().push(id.clone());
    }
    let mut names = BTreeMap::new();
    for (base, group) in by_base {
        let members: Vec<(String, u64)> = group.iter().cloned().map(|id| (id, 0)).collect();
        let ranks = collision_ranks(&members);
        for id in group {
            let rank = ranks.get(&id).copied().unwrap_or(1);
            names.insert(id, numbered(&base, rank));
        }
    }
    names
}

fn chrome<'a>(
    peer_id: &'a str,
    painted_by: Paint,
    as_of_ms: u64,
    rendered_at_ms: u64,
    names: &'a BTreeMap<String, String>,
) -> Chrome<'a> {
    Chrome {
        painted_by,
        as_of_ms,
        rendered_at_ms,
        server: peer_id,
        names,
    }
}

fn paint_of(decision: &Decision) -> Paint {
    match *decision {
        Decision::Decided { colour, .. } => Paint::Decided(colour),
        Decision::Undecided => Paint::Undecided,
    }
}

fn decision_json(decision: &Decision) -> Value {
    match *decision {
        Decision::Decided {
            colour,
            held_from_ms,
            held_to_ms,
        } => json!({
            "colour": colour,
            "held_from_ms": held_from_ms,
            "held_to_ms": held_to_ms,
        }),
        Decision::Undecided => json!("undecided"),
    }
}

fn records_json(frame: &Frame) -> DayRecords {
    let mut votes: Vec<PublishedVote> = frame
        .votes
        .iter()
        .map(|vote| PublishedVote {
            author: vote.author.clone(),
            colour: vote.colour,
            at_ms: vote.at_ms,
        })
        .collect();
    votes.sort_by(|left, right| {
        (left.at_ms, left.author.as_str(), left.colour).cmp(&(
            right.at_ms,
            right.author.as_str(),
            right.colour,
        ))
    });
    let mut arguments: Vec<PublishedArgument> = frame
        .arguments
        .iter()
        .map(|argument| PublishedArgument {
            id: argument.id.clone(),
            author: argument.author.clone(),
            at_ms: argument.at_ms,
            text: argument.text.clone(),
            cites: argument
                .cites
                .iter()
                .map(|evidence| evidence.id().0.clone())
                .collect(),
        })
        .collect();
    arguments.sort_by(|left, right| {
        (left.at_ms, left.id.as_str()).cmp(&(right.at_ms, right.id.as_str()))
    });
    let mut evidence: Vec<PublishedEvidence> = frame
        .evidence
        .iter()
        .map(|evidence| PublishedEvidence {
            id: evidence.id().0.clone(),
            author: evidence.author().to_owned(),
            url: evidence.url().to_string(),
            fetched_at_ms: evidence.fetched_at_ms(),
            sha256: evidence.sha256().to_owned(),
            excerpt: evidence.excerpt().to_owned(),
        })
        .collect();
    evidence.sort_by(|left, right| left.id.cmp(&right.id));
    DayRecords {
        votes,
        arguments,
        evidence,
    }
}

#[derive(Serialize)]
struct DayDocument {
    day: Day,
    decision: Value,
    live_voters: Vec<String>,
    rule: &'static str,
    live_rule: &'static str,
    records: DayRecords,
}

#[derive(Serialize)]
struct DayRecords {
    votes: Vec<PublishedVote>,
    arguments: Vec<PublishedArgument>,
    evidence: Vec<PublishedEvidence>,
}

#[derive(Serialize)]
struct PublishedVote {
    author: String,
    colour: Colour,
    at_ms: u64,
}

#[derive(Serialize)]
struct PublishedArgument {
    id: String,
    author: String,
    at_ms: u64,
    text: String,
    cites: Vec<String>,
}

#[derive(Serialize)]
struct PublishedEvidence {
    id: String,
    author: String,
    url: String,
    fetched_at_ms: u64,
    sha256: String,
    excerpt: String,
}

async fn read_paint(site_dir: &Path, day: Day) -> Result<Paint> {
    let path = site_dir.join(format!("days/{day}.json"));
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| Error::new("storage", error.to_string()))?;
            paint_from_decision(&value["decision"])
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Paint::NoRecord),
        Err(error) => Err(storage(error)),
    }
}

fn paint_from_decision(decision: &Value) -> Result<Paint> {
    match decision {
        Value::String(text) if text == "undecided" => Ok(Paint::Undecided),
        Value::Object(map) => {
            let name = map
                .get("colour")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new("storage", "day decision has no colour"))?;
            let colour = name
                .parse::<Colour>()
                .map_err(|_| Error::new("storage", "day decision colour is unknown"))?;
            Ok(Paint::Decided(colour))
        }
        _ => Err(Error::new("storage", "day decision is missing")),
    }
}

async fn read_archive(site_dir: &Path) -> Result<Vec<(Day, Paint)>> {
    let directory = site_dir.join("days");
    let mut read = tokio::fs::read_dir(&directory).await.map_err(storage)?;
    let mut days = Vec::new();
    while let Some(entry) = read.next_entry().await.map_err(storage)? {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = tokio::fs::read(&path).await.map_err(storage)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| Error::new("storage", error.to_string()))?;
        let day = value
            .get("day")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("storage", "day file has no day"))?
            .parse::<Day>()
            .map_err(|_| Error::new("storage", "day file has a bad day"))?;
        days.push((day, paint_from_decision(&value["decision"])?));
    }
    days.sort_by_key(|(day, _)| *day);
    Ok(days)
}

struct RunState {
    activations: Activations,
    started_real_ms: u64,
}

async fn write_status(
    config: &PeerConfig,
    day: Day,
    phase: Phase,
    index: Option<&DayIndex>,
    state: &RunState,
    previous_day: (Day, Paint),
) -> Result<()> {
    let frame = index.map(|index| frame_from(index, &config.clock));
    let authors_seen = index.map_or_else(BTreeSet::new, |index| {
        authors(index).map(ToOwned::to_owned).collect()
    });
    let empty_votes: &[Vote] = &[];
    let empty_evidence: &[Evidence] = &[];
    let votes = frame
        .as_ref()
        .map_or(empty_votes, |frame| frame.votes.as_slice());
    let evidence = frame
        .as_ref()
        .map_or(empty_evidence, |frame| frame.evidence.as_slice());
    let arguments = frame.as_ref().map_or(0, |frame| frame.arguments.len());
    let input = StatusInput {
        peer_id: &config.peer_id,
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("LISKOV_PLUMAGE_BUILD_COMMIT"),
        started_real_ms: state.started_real_ms,
        as_of_ms: forthebirds_store::now(),
        as_of_sim_ms: config.clock.now_ms(),
        day,
        phase,
        votes,
        evidence,
        arguments,
        authors_seen: &authors_seen,
        activations: &state.activations,
        previous_day,
    };
    let bytes = render_status(&input);
    if bytes.len() > MAX_STATUS_BYTES {
        return Err(Error::new(
            "storage",
            format!("status.json is {} bytes", bytes.len()),
        ));
    }
    write_atomic(&config.site_dir.join("status.json"), &bytes).await
}

async fn write_assets(site_dir: &Path) -> Result<()> {
    for (path, bytes) in site::ASSETS {
        write_atomic(&site_dir.join(path), bytes).await?;
    }
    Ok(())
}

async fn write_how(
    config: &PeerConfig,
    names: &BTreeMap<String, String>,
    paint: Paint,
) -> Result<()> {
    let now = config.clock.now_ms();
    let chrome = chrome(&config.peer_id, paint, now, now, names);
    write_atomic(
        &config.site_dir.join("how.html"),
        render_how(&chrome).as_bytes(),
    )
    .await
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await.map_err(storage)?;
    }
    let temporary = temporary_path(path);
    {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::File::create(&temporary).await.map_err(storage)?;
        file.write_all(bytes).await.map_err(storage)?;
        file.sync_all().await.map_err(storage)?;
    }
    tokio::fs::rename(&temporary, path).await.map_err(storage)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// `true` when `shutdown` is set or its sender is gone.
async fn wait_until(clock: &DayClock, sim_ms: u64, shutdown: &mut watch::Receiver<bool>) -> bool {
    loop {
        if halted(shutdown) {
            return true;
        }
        let now = forthebirds_store::now();
        let wake = clock.real(sim_ms);
        if now >= wake {
            return false;
        }
        let delay = Duration::from_millis(wake.saturating_sub(now));
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || halted(shutdown) {
                    return true;
                }
            }
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

fn halted(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

fn storage(error: std::io::Error) -> Error {
    Error::new("storage", error.to_string())
}
