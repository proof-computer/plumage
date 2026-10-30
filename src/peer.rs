//! One peer process: the open day and the last seven closed days, pinned sync,
//! activations, and the site.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use forthebirds_inference::InferenceConfig;
use forthebirds_inference::model::Model;
use forthebirds_names::{bird_name, collision_ranks, numbered};
use forthebirds_net::{
    PeerTarget, PeerTransport, StaticPeer, StaticPeers, TransportConfig, TransportState,
};
use forthebirds_runtime::activation::Limits;
use forthebirds_store::LedgerConfig;
use forthebirds_sync::{Replicator, Runs, SyncConfig};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, watch};
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
use crate::record::Record;
use crate::site::{
    self, Activations, ArchivePage, ArgumentView, Chrome, DayPage, MAX_STATUS_BYTES, Paint, Phase,
    StatusInput, render_archive, render_day, render_flock, render_front, render_how, render_status,
};
use crate::tally;

/// Closed day ledgers kept beside the open day.
pub const KEEP_CLOSED_DAYS: u32 = 7;

/// Page cache for a closed day's database. The open day keeps
/// [`DATABASE_CACHE_BYTES`], so eight open books stay within 4 MiB + 7 × 1 MiB.
const CLOSED_DAY_CACHE_BYTES: usize = 1024 * 1024;

type DayReplicator = Replicator<Book, Record, ()>;

/// Where each sync tick reads the other peers.
///
/// `StaticPeers` is not `Clone`, and this crate does not change forthebirds,
/// so the static list is shared through `Arc`.
#[derive(Clone)]
pub enum PeerSource {
    Static(Arc<StaticPeers>),
    Liskov(crate::liskov::LiskovPeers),
    /// Targets built by [`crate::liskov::directory_target`], including loopback
    /// addresses the directory file reader refuses.
    Mapped(MappedPeers),
}

/// One sync tick's targets. The closure runs on every tick.
#[derive(Clone)]
pub struct MappedPeers {
    read: Arc<dyn Fn() -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> + Send + Sync>,
}

impl PeerSource {
    /// Today's static list. An invalid entry is `config` before the day starts.
    pub fn from_static(peers: Vec<StaticPeer>) -> Result<Self> {
        Ok(Self::Static(Arc::new(StaticPeers::new(peers)?)))
    }

    /// Re-read the helper directory on each tick, at most once a second.
    pub fn liskov(env: liskov_cargo_harness::directory::RuntimeEnv) -> Self {
        Self::Liskov(crate::liskov::LiskovPeers::new(env))
    }

    /// Call `read` on every tick. The exchange test uses this for loopback.
    pub fn mapped(
        read: impl Fn() -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> + Send + Sync + 'static,
    ) -> Self {
        Self::Mapped(MappedPeers {
            read: Arc::new(read),
        })
    }

    /// The map `Replicator::tick` takes. Keys are plumage peer ids.
    pub fn read(&self) -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> {
        match self {
            Self::Static(peers) => peers.read(),
            Self::Liskov(peers) => peers.read(),
            Self::Mapped(peers) => (peers.read)(),
        }
    }
}

/// Run from `config.clock.day()` through `last_day`. `shutdown` returns `Ok`.
///
/// `peers` is read on every sync tick of every kept day. The fetch `source`
/// is unchanged.
pub async fn run<S: Source + 'static>(
    config: PeerConfig,
    source: S,
    peers: PeerSource,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    config.validate()?;
    let config = Arc::new(config);
    let state = RunState {
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
    let day = config.clock.day();
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
    let runs = Runs::new(&config.peer_id, config.token.clone())?;
    let router = runs.router();
    let transport = PeerTransport::start(
        &TransportConfig {
            listen: config.listen,
            advertisement_file: Some(config.advertisement_file.clone()),
            service: "plumage".to_owned(),
            application: "plumage".to_owned(),
        },
        &TransportState::default(),
        router,
    )
    .await?;
    let shared = Arc::new(Shared {
        config: Arc::clone(&config),
        peers,
        runs,
        members: Mutex::new(BTreeMap::new()),
        closed_here: Mutex::new(BTreeSet::new()),
        open_day: Mutex::new(day),
        state: Mutex::new(state),
        site: Mutex::new(()),
        error: Mutex::new(None),
        error_notify: Notify::new(),
    });
    let outcome = run_days(&config, &model, &source, &shared, day, &mut shutdown).await;
    shared.stop_all().await;
    let stopped = transport.stop().await;
    if let Some(error) = shared.error.lock().await.take() {
        return Err(error);
    }
    outcome?;
    stopped?;
    Ok(())
}

struct Member {
    book: Book,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

struct Shared {
    config: Arc<PeerConfig>,
    peers: PeerSource,
    runs: Runs<Book, Record, ()>,
    members: Mutex<BTreeMap<Day, Member>>,
    closed_here: Mutex<BTreeSet<Day>>,
    open_day: Mutex<Day>,
    state: Mutex<RunState>,
    site: Mutex<()>,
    error: Mutex<Option<Error>>,
    error_notify: Notify,
}

impl Shared {
    async fn insert(self: &Arc<Self>, day: Day, cache: usize) -> Result<()> {
        let book = Book::open(ledger_config(&self.config, day, cache)).await?;
        let replicator = Replicator::new(
            book.clone(),
            SyncConfig {
                run_id: day.to_string(),
                peer_id: self.config.peer_id.clone(),
                token: self.config.token.clone(),
                request_timeout: Duration::from_secs(self.config.request_timeout_seconds),
            },
        )?;
        self.runs.insert(replicator.clone()).await?;
        let (stop, stop_rx) = watch::channel(false);
        let shared = Arc::clone(self);
        let task_book = book.clone();
        let task = tokio::spawn(async move {
            sync_day(shared, replicator, task_book, day, stop_rx).await;
        });
        self.members
            .lock()
            .await
            .insert(day, Member { book, stop, task });
        Ok(())
    }

    async fn note_closed(&self, day: Day) {
        self.closed_here.lock().await.insert(day);
    }

    /// Stop the day's loop, drop its book, and wait until the database file is free.
    async fn stop_member(&self, day: Day) -> Result<()> {
        let Some(member) = self.members.lock().await.remove(&day) else {
            return Ok(());
        };
        let Member { book, stop, task } = member;
        let _ = stop.send(true);
        let _ = task.await;
        let _ = self.runs.remove(&day.to_string()).await;
        let db = Arc::clone(&book.ledger().db);
        drop(book);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&db) > 1 {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::new(
                    "storage",
                    format!("day {day} database stayed open"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(db);
        Ok(())
    }

    /// Reopen a day that just closed so its cache is [`CLOSED_DAY_CACHE_BYTES`].
    async fn shrink(self: &Arc<Self>, day: Day) -> Result<()> {
        self.note_closed(day).await;
        self.stop_member(day).await?;
        self.insert(day, CLOSED_DAY_CACHE_BYTES).await
    }

    async fn stop_all(&self) {
        let days: Vec<Day> = self.members.lock().await.keys().copied().collect();
        for day in days {
            let _ = self.stop_member(day).await;
        }
    }

    async fn book(&self, day: Day) -> Result<Book> {
        self.members
            .lock()
            .await
            .get(&day)
            .map(|member| member.book.clone())
            .ok_or_else(|| Error::new("storage", format!("day {day} is not open")))
    }

    async fn fail(&self, error: Error) {
        let mut slot = self.error.lock().await;
        if slot.is_none() {
            *slot = Some(error);
            self.error_notify.notify_one();
        }
    }

    async fn settled(&self, day: Day, replicator: &DayReplicator) -> bool {
        if self.closed_here.lock().await.contains(&day) {
            return true;
        }
        replicator.caught_up_ms().await.is_some()
    }

    async fn maybe_recompute(
        &self,
        replicator: &DayReplicator,
        book: &Book,
        day: Day,
    ) -> Result<()> {
        let open = *self.open_day.lock().await;
        let closed_here = self.closed_here.lock().await.contains(&day);
        if day >= open && !closed_here {
            return Ok(());
        }
        if !closed_here && replicator.caught_up_ms().await.is_none() {
            return Ok(());
        }
        if record_count(book) == 0 {
            return Ok(());
        }
        let _site = self.site.lock().await;
        let old_paint = read_paint(&self.config.site_dir, day).await?;
        let closed = write_closed_day(&self.config, book, day).await?;
        if closed.json_changed {
            println!(
                "plumage day day={day} peer={} decision={} live={} records={} source=recomputed",
                self.config.peer_id, closed.label, closed.live, closed.records
            );
        }
        if day == open.previous() && old_paint != closed.paint {
            let open_book = self.book(open).await?;
            write_live(&self.config, &open_book, open).await?;
            let names = display_names(&self.config.peer_id, std::iter::empty());
            write_how(&self.config, &names, closed.paint).await?;
            let phase = if self.config.clock.now_ms() >= open.close_ms() {
                Phase::SyncOnly
            } else {
                Phase::Voting
            };
            let state = self.state.lock().await;
            let index = open_book.index();
            write_status(
                &self.config,
                open,
                phase,
                Some(&index),
                &state,
                (day, closed.paint),
            )
            .await?;
        }
        Ok(())
    }
}

/// Same interval, targets and shutdown as [`Replicator::run`]. The tick is
/// explicit so a closed day is recomputed only after that tick sets
/// `caught_up_ms`.
async fn sync_day(
    shared: Arc<Shared>,
    replicator: DayReplicator,
    book: Book,
    day: Day,
    mut shutdown: watch::Receiver<bool>,
) {
    let peers = shared.peers.clone();
    let every = Duration::from_millis(shared.config.sync_interval_ms);
    let mut seen = record_count(&book);
    let mut was_settled = shared.settled(day, &replicator).await;
    if let Err(error) = shared.maybe_recompute(&replicator, &book, day).await {
        shared.fail(error).await;
        return;
    }
    let mut interval = tokio::time::interval(every);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
        }
        if *shutdown.borrow() {
            return;
        }
        replicator.tick(peers.read(), &mut shutdown).await;
        let count = record_count(&book);
        let settled = shared.settled(day, &replicator).await;
        let gained = count > seen;
        let became = settled && !was_settled;
        seen = count;
        was_settled = settled;
        if settled
            && count > 0
            && (gained || became)
            && let Err(error) = shared.maybe_recompute(&replicator, &book, day).await
        {
            shared.fail(error).await;
            return;
        }
    }
}

async fn run_days<S: Source>(
    config: &PeerConfig,
    model: &Model,
    source: &S,
    shared: &Arc<Shared>,
    mut day: Day,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<()> {
    for age in (0..=KEEP_CLOSED_DAYS).rev() {
        let cache = if age == 0 {
            DATABASE_CACHE_BYTES
        } else {
            CLOSED_DAY_CACHE_BYTES
        };
        shared.insert(rewind(day, age), cache).await?;
    }
    loop {
        if let Some(error) = shared.error.lock().await.take() {
            return Err(error);
        }
        if halted(shutdown) {
            return Ok(());
        }
        if let Some(last) = config.last_day
            && day > last
        {
            return Ok(());
        }
        *shared.open_day.lock().await = day;
        let stop = run_day(config, model, source, day, shutdown, shared).await?;
        if stop || config.last_day == Some(day) {
            return Ok(());
        }
        let next = day.next();
        shared.shrink(day).await?;
        shared.insert(next, DATABASE_CACHE_BYTES).await?;
        shared.stop_member(rewind(day, KEEP_CLOSED_DAYS)).await?;
        day = next;
    }
}

/// `true` when shutdown asked the loop to stop.
async fn run_day<S: Source>(
    config: &PeerConfig,
    model: &Model,
    source: &S,
    day: Day,
    shutdown: &mut watch::Receiver<bool>,
    shared: &Arc<Shared>,
) -> Result<bool> {
    let book = shared.book(day).await?;
    let limits = Limits {
        max_steps: usize::try_from(config.max_steps).unwrap_or(usize::MAX),
        output_tokens: config.output_tokens,
        continuation: false,
    };
    let mut last_start = None;
    loop {
        if let Some(error) = shared.error.lock().await.take() {
            return Err(error);
        }
        if halted(shutdown) {
            return Ok(true);
        }
        let now = config.clock.now_ms();
        if now >= day.close_ms() {
            let previous = day.previous();
            let paint = read_paint(&config.site_dir, previous).await?;
            let index = book.index();
            let _site = shared.site.lock().await;
            let state = shared.state.lock().await;
            write_status(
                config,
                day,
                Phase::SyncOnly,
                Some(&index),
                &state,
                (previous, paint),
            )
            .await?;
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
            if wait_until(&config.clock, wake, shutdown, shared).await {
                if let Some(error) = shared.error.lock().await.take() {
                    return Err(error);
                }
                return Ok(true);
            }
            continue;
        }
        last_start = Some(now);
        let id = format!("a{now}");
        let mut fetch_failures = Vec::new();
        match activation::activate(
            &book,
            model,
            source,
            &config.peer_id,
            day,
            config.clock,
            limits,
            &id,
            &mut fetch_failures,
        )
        .await
        {
            Ok(colour) => {
                shared
                    .state
                    .lock()
                    .await
                    .activations
                    .record_vote(colour, forthebirds_store::now());
                println!(
                    "plumage activation day={day} peer={} id={id} outcome=voted:{}",
                    config.peer_id,
                    colour.name()
                );
            }
            Err(error) => {
                shared
                    .state
                    .lock()
                    .await
                    .activations
                    .record_error(error.code, forthebirds_store::now());
                println!(
                    "plumage activation day={day} peer={} id={id} outcome=error:{}",
                    config.peer_id, error.code
                );
            }
        }
        for failure in &fetch_failures {
            shared
                .state
                .lock()
                .await
                .activations
                .record_fetch_failure(failure.code);
            println!(
                "plumage fetch day={day} peer={} id={id} host={} outcome=error:{}",
                config.peer_id, failure.host, failure.code
            );
        }
        let previous = day.previous();
        let paint = read_paint(&config.site_dir, previous).await?;
        let index = book.index();
        {
            let _site = shared.site.lock().await;
            write_live(config, &book, day).await?;
            let state = shared.state.lock().await;
            write_status(
                config,
                day,
                Phase::Voting,
                Some(&index),
                &state,
                (previous, paint),
            )
            .await?;
        }
    }
    if wait_until(&config.clock, day.next().open_ms(), shutdown, shared).await {
        if let Some(error) = shared.error.lock().await.take() {
            return Err(error);
        }
        return Ok(true);
    }
    {
        let _site = shared.site.lock().await;
        let state = shared.state.lock().await;
        publish_close(config, &book, day, &state).await?;
    }
    shared.note_closed(day).await;
    drop(book);
    Ok(false)
}

fn ledger_config(config: &PeerConfig, day: Day, cache: usize) -> LedgerConfig {
    LedgerConfig {
        database_path: config.data_dir.join(format!("{day}.redb")),
        database_cache_bytes: cache,
        run_id: day.to_string(),
        peer_id: config.peer_id.clone(),
        token_budget: config.token_budget,
        memory_budget: config.memory_budget,
        memory_item_tokens: config.memory_item_tokens,
        max_documents: MAX_DOCUMENTS,
    }
}

fn rewind(mut day: Day, days: u32) -> Day {
    for _ in 0..days {
        day = day.previous();
    }
    day
}

fn record_count(book: &Book) -> usize {
    let index = book.index();
    index.votes.len() + index.arguments.len() + index.evidence.len()
}

async fn write_live(config: &PeerConfig, book: &Book, day: Day) -> Result<()> {
    let index = book.index();
    let painted_by = read_paint(&config.site_dir, day.previous()).await?;
    let now = config.clock.now_ms();
    let frame = frame_from(&index, &config.clock);
    let names = display_names(&config.peer_id, authors(&index));
    let chrome = chrome(&config.peer_id, painted_by, now, now, &names);
    let archive = read_archive(&config.site_dir).await?;
    let (previous, next) = neighbours(&archive, day, Day::from_ms(now));
    let page = DayPage {
        day,
        previous,
        next,
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
    write_atomic(
        &config.site_dir.join("days/index.html"),
        render_archive(&ArchivePage {
            days: &archive,
            chrome,
        })
        .as_bytes(),
    )
    .await?;
    Ok(())
}

struct ClosedDay {
    paint: Paint,
    label: &'static str,
    live: usize,
    records: usize,
    json_changed: bool,
}

/// `days/<C>.json`, `days/<C>.html` and `days/index.html` from the book's votes.
///
/// The JSON is left untouched when it would not change, so a later import that
/// repeats the same records does not rewrite the day.
async fn write_closed_day(config: &PeerConfig, book: &Book, day: Day) -> Result<ClosedDay> {
    let index = book.index();
    let frame = frame_from(&index, &config.clock);
    let open = day.open_ms();
    let close = day.close_ms();
    let live = closing::live_voters(&frame.votes, open, close);
    let decision = closing::decide(&frame.votes, &live, open, close);
    let now = config.clock.now_ms();
    let names = display_names(&config.peer_id, authors(&index));
    let paint = paint_of(&decision);
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
    let path = config.site_dir.join(format!("days/{day}.json"));
    let previous = match tokio::fs::read(&path).await {
        Ok(existing) => Some(existing),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(storage(error)),
    };
    let json_changed = previous.as_deref() != Some(published.as_slice());
    if json_changed {
        write_atomic(&path, &published).await?;
        let archive = read_archive(&config.site_dir).await?;
        let (previous, next) = neighbours(&archive, day, Day::from_ms(now));
        let closed = DayPage {
            day,
            previous,
            next,
            votes: &frame.votes,
            arguments: &frame.arguments,
            evidence: &frame.evidence,
            decision: Some(decision),
            chrome: chrome(&config.peer_id, paint, now, now, &names),
        };
        write_atomic(
            &config.site_dir.join(format!("days/{day}.html")),
            render_day(&closed).as_bytes(),
        )
        .await?;
        let archive_names = display_names(&config.peer_id, std::iter::empty());
        let archive_chrome = chrome(&config.peer_id, paint, now, now, &archive_names);
        write_atomic(
            &config.site_dir.join("days/index.html"),
            render_archive(&ArchivePage {
                days: &archive,
                chrome: archive_chrome,
            })
            .as_bytes(),
        )
        .await?;
    }
    let label = match decision {
        Decision::Decided { colour, .. } => colour.name(),
        Decision::Undecided => "undecided",
    };
    Ok(ClosedDay {
        paint,
        label,
        live: live.len(),
        records: index.votes.len() + index.arguments.len() + index.evidence.len(),
        json_changed,
    })
}

async fn publish_close(config: &PeerConfig, book: &Book, day: Day, state: &RunState) -> Result<()> {
    let closed = write_closed_day(config, book, day).await?;
    let now = config.clock.now_ms();
    let next = day.next();
    let today_names = display_names(&config.peer_id, std::iter::empty());
    let today = DayPage {
        day: next,
        previous: None,
        next: None,
        votes: &[],
        arguments: &[],
        evidence: &[],
        decision: None,
        chrome: chrome(&config.peer_id, closed.paint, now, now, &today_names),
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
    write_how(config, &today_names, closed.paint).await?;

    println!(
        "plumage day day={day} peer={} decision={} live={} records={}",
        config.peer_id, closed.label, closed.live, closed.records
    );
    let index = book.index();
    write_status(
        config,
        day,
        Phase::Closed,
        Some(&index),
        state,
        (day, closed.paint),
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

/// Replies whose target is an argument in this index, timed on the day clock.
///
/// An unresolved `reply_to` is skipped. The status write does not fail.
fn replies_from(index: &DayIndex, clock: &DayClock) -> Vec<tally::Reply> {
    let authors: BTreeMap<&str, &str> = index
        .arguments
        .iter()
        .map(|argument| (argument.id.as_str(), argument.author.as_str()))
        .collect();
    index
        .arguments
        .iter()
        .filter_map(|argument| {
            let target = argument.reply_to.as_deref()?;
            Some(tally::Reply {
                at_ms: clock.sim(argument.at_ms),
                target_author: authors.get(target)?.to_string(),
            })
        })
        .collect()
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
    let mut read = match tokio::fs::read_dir(&directory).await {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(storage(error)),
    };
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

type Neighbours = (Option<(Day, Paint)>, Option<(Day, Paint)>);

/// Nearest published closed days, with the open day as the final next page.
fn neighbours(archive: &[(Day, Paint)], day: Day, today: Day) -> Neighbours {
    let previous = archive.iter().rev().find(|(date, _)| *date < day).copied();
    let next = if day < today {
        archive
            .iter()
            .find(|(date, _)| *date > day && *date < today)
            .copied()
            .or(Some((today, Paint::NoRecord)))
    } else {
        None
    };
    (previous, next)
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
    let replies = index.map_or_else(Vec::new, |index| replies_from(index, &config.clock));
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
        replies: &replies,
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
async fn wait_until(
    clock: &DayClock,
    sim_ms: u64,
    shutdown: &mut watch::Receiver<bool>,
    shared: &Shared,
) -> bool {
    loop {
        if halted(shutdown) || shared.error.lock().await.is_some() {
            return true;
        }
        let now = forthebirds_store::now();
        let wake = clock.real(sim_ms);
        if now >= wake {
            return false;
        }
        let delay = Duration::from_millis(wake.saturating_sub(now).min(200));
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || halted(shutdown) {
                    return true;
                }
            }
            _ = tokio::time::sleep(delay) => {}
            _ = shared.error_notify.notified() => return true,
        }
    }
}

fn halted(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

fn storage(error: std::io::Error) -> Error {
    Error::new("storage", error.to_string())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn neighbours_skip_gaps_and_stop_at_the_open_day() {
        let first: Day = "2026-10-21".parse().unwrap();
        let middle: Day = "2026-10-24".parse().unwrap();
        let last: Day = "2026-10-26".parse().unwrap();
        let today: Day = "2026-10-30".parse().unwrap();
        let blue = Paint::Decided(Colour::Blue);
        let archive = [(first, blue), (middle, Paint::Undecided), (last, blue)];
        assert_eq!(neighbours(&[], today, today), (None, None));
        assert_eq!(
            neighbours(&archive, first, today),
            (None, Some((middle, Paint::Undecided)))
        );
        assert_eq!(
            neighbours(&archive, middle, today),
            (Some((first, blue)), Some((last, blue)))
        );
        assert_eq!(
            neighbours(&archive, middle.next(), today),
            (Some((middle, Paint::Undecided)), Some((last, blue)))
        );
        assert_eq!(
            neighbours(&archive, last, today),
            (
                Some((middle, Paint::Undecided)),
                Some((today, Paint::NoRecord))
            )
        );
        assert_eq!(
            neighbours(&archive, today, today),
            (Some((last, blue)), None)
        );
    }

    #[tokio::test]
    async fn archive_reads_missing_days_as_empty_but_preserves_other_errors() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_archive(root.path()).await.unwrap().is_empty());

        std::fs::write(root.path().join("days"), b"not a directory").unwrap();
        assert_eq!(read_archive(root.path()).await.unwrap_err().code, "storage");
    }

    #[tokio::test]
    async fn a_reply_then_a_vote_change_is_counted_in_status() {
        let root = tempfile::tempdir().unwrap();
        let day = Day::from_str("2026-10-26").unwrap();
        let config = PeerConfig {
            peer_id: "a".to_owned(),
            data_dir: root.path().join("data"),
            site_dir: root.path().join("site"),
            model_url: "http://127.0.0.1:9/".to_owned(),
            listen: "[::1]:0".parse().unwrap(),
            advertisement_file: root.path().join("a.ad"),
            peers: Vec::new(),
            token: "fixture-only-peer-token-0123456789".to_owned(),
            clock: DayClock {
                real_start_ms: 0,
                sim_start_ms: 0,
                speed: 1,
            },
            last_day: Some(day),
            activation_interval_ms: 1_800_000,
            sync_interval_ms: 100,
            max_steps: 16,
            context_tokens: 4_096,
            output_tokens: 512,
            inference_timeout_seconds: 180,
            request_timeout_seconds: 30,
            fetch_timeout_seconds: 10,
            token_budget: None,
            memory_budget: 1_000_000,
            memory_item_tokens: 512,
            liskov: None,
        };
        let mut index = DayIndex::default();
        index.arguments = vec![
            Argument {
                id: "arg-b".to_owned(),
                author: "b".to_owned(),
                at_ms: 10,
                text: "blue holds".to_owned(),
                cites: Vec::new(),
                reply_to: None,
            },
            Argument {
                id: "arg-a".to_owned(),
                author: "a".to_owned(),
                at_ms: 20,
                text: "answering b".to_owned(),
                cites: Vec::new(),
                reply_to: Some("arg-b".to_owned()),
            },
            Argument {
                id: "arg-missing".to_owned(),
                author: "a".to_owned(),
                at_ms: 21,
                text: "names nothing stored".to_owned(),
                cites: Vec::new(),
                reply_to: Some("arg-absent".to_owned()),
            },
        ];
        index.votes = vec![
            Vote {
                author: "b".to_owned(),
                colour: Colour::Blue,
                at_ms: 5,
            },
            Vote {
                author: "b".to_owned(),
                colour: Colour::Red,
                at_ms: 30,
            },
        ];
        let state = RunState {
            activations: Activations::default(),
            started_real_ms: 0,
        };
        write_status(
            &config,
            day,
            Phase::Voting,
            Some(&index),
            &state,
            (day.previous(), Paint::NoRecord),
        )
        .await
        .unwrap();

        let bytes = std::fs::read(config.site_dir.join("status.json")).unwrap();
        let rendered = String::from_utf8(bytes.clone()).unwrap();
        assert!(!rendered.contains("blue holds"));
        assert!(!rendered.contains("answering b"));
        assert!(!rendered.contains("names nothing stored"));
        let status: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status["schema"], "plumage.status.v1");
        assert_eq!(status["records"]["replies"], 1);
        assert_eq!(status["reply_led_changes"], 1);
    }
}
