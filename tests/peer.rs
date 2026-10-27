//! A whole simulated day, in process, against the stateless fake model.

use std::future::Future;
use std::net::Ipv6Addr;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;

use forthebirds_net::{StaticPeer, pinned_client};
use forthebirds_store::LedgerConfig;
use forthebirds_sync::{CATCH_UP_PAGES, ROUTE};
use plumage::actions::Action;
use plumage::book::Book;
use plumage::clock::DayClock;
use plumage::colour::Colour;
use plumage::config::PeerConfig;
use plumage::day::Day;
use plumage::fetch::{FetchError, Source};
use plumage::peer;
use plumage::peer::PeerSource;
use plumage::site::ASSETS;
use serde_json::{Value, json};
use tokio::sync::watch;
use url::Url;

#[path = "support/mod.rs"]
mod support;

const TOKEN: &str = "fixture-only-peer-token-0123456789";
const DAY: &str = "2026-10-26";

struct EmptySource;

impl Source for EmptySource {
    fn get(&self, url: &Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
        let url = url.clone();
        async move {
            Err(FetchError {
                code: "fetch_failed",
                message: format!("test source refuses {url}"),
            })
        }
    }
}

fn wall_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn day() -> Day {
    Day::from_str(DAY).unwrap()
}

fn clock() -> DayClock {
    clock_at(2880)
}

fn clock_at(speed: u64) -> DayClock {
    DayClock {
        real_start_ms: wall_now(),
        sim_start_ms: day().open_ms(),
        speed,
    }
}

fn shift(mut day: Day, steps: i32) -> Day {
    if steps >= 0 {
        for _ in 0..steps {
            day = day.next();
        }
    } else {
        for _ in 0..(-steps) {
            day = day.previous();
        }
    }
    day
}

fn peer_config(
    root: &Path,
    id: &str,
    model_url: &str,
    peers: Vec<StaticPeer>,
    clock: DayClock,
) -> PeerConfig {
    PeerConfig {
        peer_id: id.to_owned(),
        data_dir: root.join(id),
        site_dir: root.join(format!("{id}-site")),
        model_url: model_url.to_owned(),
        listen: "[::1]:0".parse().unwrap(),
        advertisement_file: root.join(format!("{id}.ad")),
        peers,
        token: TOKEN.to_owned(),
        clock,
        last_day: Some(day()),
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
    }
}

fn start(
    config: PeerConfig,
    shutdown: watch::Receiver<bool>,
) -> impl Future<Output = Result<(), plumage::error::Error>> {
    let peers = PeerSource::from_static(config.peers.clone()).expect("static peers");
    peer::run(config, EmptySource, peers, shutdown)
}

fn required_config() -> Value {
    json!({
        "peer_id": "p00",
        "data_dir": "data",
        "site_dir": "site",
        "model_url": "http://127.0.0.1:9/",
        "listen": "[::1]:9743",
        "advertisement_file": "p00.ad",
        "peers": [],
        "token": TOKEN,
        "clock": {"real_start_ms": 0, "sim_start_ms": 0, "speed": 1},
        "last_day": null
    })
}

#[test]
fn peer_config_refuses_ipv4_a_short_token_and_speed_zero_and_fills_defaults() {
    let mut ipv4 = required_config();
    ipv4["listen"] = json!("127.0.0.1:9");
    let error = serde_json::from_value::<PeerConfig>(ipv4)
        .unwrap()
        .validate()
        .unwrap_err();
    assert_eq!(error.code, "config");

    let mut short = required_config();
    let token = "0123456789012345678901234567890";
    assert_eq!(token.len(), 31);
    short["token"] = json!(token);
    let error = serde_json::from_value::<PeerConfig>(short)
        .unwrap()
        .validate()
        .unwrap_err();
    assert_eq!(error.code, "config");

    let mut stopped = required_config();
    stopped["clock"]["speed"] = json!(0);
    let error = serde_json::from_value::<PeerConfig>(stopped)
        .unwrap()
        .validate()
        .unwrap_err();
    assert_eq!(error.code, "config");

    let config: PeerConfig = serde_json::from_value(required_config()).unwrap();
    assert!(config.validate().is_ok());
    assert_eq!(
        config.activation_interval_ms,
        forthebirds_runtime::activation::ACTIVATION_INTERVAL_MS
    );
    assert_eq!(config.sync_interval_ms, 5_000);
    assert_eq!(config.max_steps, 16);
    assert_eq!(config.context_tokens, 4_096);
    assert_eq!(config.output_tokens, 512);
    assert_eq!(config.inference_timeout_seconds, 180);
    assert_eq!(config.request_timeout_seconds, 30);
    assert_eq!(config.fetch_timeout_seconds, 10);
    assert_eq!(config.token_budget, None);
    assert_eq!(config.memory_budget, 1_000_000);
    assert_eq!(config.memory_item_tokens, 512);
}

#[test]
fn binary_with_no_arguments_prints_the_usage_and_exits_0() {
    let output = Command::new(env!("CARGO_BIN_EXE_plumage"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(&format!("plumage {}", env!("CARGO_PKG_VERSION"))));
    assert!(text.contains("plumage peer <config.json>"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_peer_votes_blue_and_paints_the_next_days_front_page() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let shared = clock();
    let (sender, receiver) = watch::channel(false);
    let config = peer_config(root.path(), "p00", &model, Vec::new(), shared);
    let started = wall_now();
    start(config, receiver).await.unwrap();
    let ended = wall_now();
    drop(sender);
    server.abort();

    let site = root.path().join("p00-site");
    let document: Value =
        serde_json::from_slice(&std::fs::read(site.join("days/2026-10-26.json")).unwrap()).unwrap();
    assert_eq!(document["day"], DAY);
    assert_eq!(document["decision"]["colour"], "blue");
    assert_eq!(document["live_voters"], json!(["p00"]));
    let open = day().open_ms();
    let close = day().close_ms();
    let from = document["decision"]["held_from_ms"].as_u64().unwrap();
    let to = document["decision"]["held_to_ms"].as_u64().unwrap();
    assert!(open <= from && to <= close && from < to);

    let day_html = std::fs::read_to_string(site.join("days/2026-10-26.html")).unwrap();
    assert!(day_html.contains("background:#0000FF"));
    let index = std::fs::read_to_string(site.join("index.html")).unwrap();
    assert!(index.contains("27 OCTOBER 2026"));
    assert!(index.contains("background:#0000FF"));
    assert!(index.contains("Garganey"));
    assert!(site.join("flock.html").is_file());
    assert!(site.join("how.html").is_file());
    let archive = std::fs::read_to_string(site.join("days/index.html")).unwrap();
    assert!(archive.contains("href=\"2026-10-26.html\""));
    for (path, bytes) in ASSETS {
        assert_eq!(std::fs::read(site.join(path)).unwrap(), bytes);
    }

    let status_bytes = std::fs::read(site.join("status.json")).unwrap();
    assert!(status_bytes.len() <= 16_384);
    let status: Value = serde_json::from_slice(&status_bytes).unwrap();
    assert_eq!(status["schema"], "plumage.status.v1");
    assert!(status["fetch_failures_by_code"].is_object());
    assert_eq!(status["phase"], "closed");
    assert_eq!(status["day"], DAY);
    assert_eq!(status["previous_day"]["paint"], "decided:blue");
    assert!(status["activations"]["voted"].as_u64().unwrap() >= 1);
    assert_eq!(status["activations"]["errors_by_code"], json!({}));
    assert_eq!(status["activations"]["recent"]["errors"], 0);
    assert!(status["activations"]["recent"]["total"].as_u64().unwrap() >= 1);
    assert_eq!(status["authors_seen"], json!(["p00"]));
    assert!(status["build"].get("commit").unwrap().is_null());
    assert_eq!(status["day_open_ms"].as_u64().unwrap(), day().open_ms());
    let as_of = status["as_of_ms"].as_u64().unwrap();
    assert!(started <= as_of && as_of <= ended);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_peers_replicate_votes_and_stay_undecided_on_a_tie() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let shared = clock();
    let left = peer_config(
        root.path(),
        "p00",
        &model,
        vec![StaticPeer {
            name: "p01".to_owned(),
            address: Ipv6Addr::LOCALHOST,
            advertisement_file: root.path().join("p01.ad"),
        }],
        shared,
    );
    let right = peer_config(
        root.path(),
        "p01",
        &model,
        vec![StaticPeer {
            name: "p00".to_owned(),
            address: Ipv6Addr::LOCALHOST,
            advertisement_file: root.path().join("p00.ad"),
        }],
        shared,
    );
    let (_left_stop, left_rx) = watch::channel(false);
    let (_right_stop, right_rx) = watch::channel(false);
    let (left_run, right_run) = tokio::join!(start(left, left_rx), start(right, right_rx),);
    left_run.unwrap();
    right_run.unwrap();
    server.abort();

    let left_bytes = std::fs::read(root.path().join("p00-site/days/2026-10-26.json")).unwrap();
    let right_bytes = std::fs::read(root.path().join("p01-site/days/2026-10-26.json")).unwrap();
    assert_eq!(left_bytes, right_bytes);
    let document: Value = serde_json::from_slice(&left_bytes).unwrap();
    assert_eq!(document["decision"], "undecided");
    let voters = document["live_voters"].as_array().unwrap();
    assert_eq!(voters.len(), 2);
    assert!(voters.contains(&json!("p00")));
    assert!(voters.contains(&json!("p01")));
    let authors: Vec<&str> = document["records"]["votes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|vote| vote["author"].as_str())
        .collect();
    assert!(authors.contains(&"p00"));
    assert!(authors.contains(&"p01"));
    for id in ["p00", "p01"] {
        let status: Value = serde_json::from_slice(
            &std::fs::read(root.path().join(format!("{id}-site/status.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(status["authors_seen"], json!(["p00", "p01"]));
    }
}

async fn until(label: &str, mut ready: impl FnMut() -> bool) {
    let start = tokio::time::Instant::now();
    while !ready() {
        if start.elapsed() > Duration::from_secs(90) {
            panic!("timed out waiting for {label}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn seed_votes(config: &PeerConfig, day: Day, count: usize) {
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let book = Book::open(LedgerConfig {
        database_path: config.data_dir.join(format!("{day}.redb")),
        database_cache_bytes: 1024 * 1024,
        run_id: day.to_string(),
        peer_id: config.peer_id.clone(),
        token_budget: config.token_budget,
        memory_budget: config.memory_budget,
        memory_item_tokens: config.memory_item_tokens,
        max_documents: plumage::config::MAX_DOCUMENTS,
    })
    .await
    .unwrap();
    for index in 0..count {
        book.write(
            &format!("s{index}"),
            Action::Vote {
                colour: Colour::Blue,
            },
            0,
        )
        .await
        .unwrap();
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

async fn post_exchange(ad: &Path, run: &str) -> (u16, Value) {
    let advertisement = read_json(ad);
    let pin = advertisement["tlsFingerprint"].as_str().unwrap();
    let port = advertisement["port"].as_u64().unwrap();
    let client = pinned_client(pin, Duration::from_secs(5)).unwrap();
    let response = client
        .client
        .post(format!("https://[::1]:{port}{ROUTE}"))
        .bearer_auth(TOKEN)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(
            serde_json::to_vec(&json!({
                "protocol": 1,
                "run_id": run,
                "peer_id": "p01",
                "records": [],
                "since": 0,
                "database_id": null
            }))
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    (status, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_incoming_job_recomputes_yesterday_from_the_sibling_ledger() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let shared = clock_at(12_000);
    let mut leader = peer_config(root.path(), "p00", &model, Vec::new(), shared);
    leader.last_day = Some(shift(day(), 1));
    let follower_peers = vec![StaticPeer {
        name: "p00".to_owned(),
        address: Ipv6Addr::LOCALHOST,
        advertisement_file: root.path().join("p00.ad"),
    }];
    leader.peers = vec![StaticPeer {
        name: "p01".to_owned(),
        address: Ipv6Addr::LOCALHOST,
        advertisement_file: root.path().join("p01.ad"),
    }];
    let (stop_leader, leader_rx) = watch::channel(false);
    let leader_run = tokio::spawn(async move { start(leader, leader_rx).await });
    let closed = root.path().join(format!("p00-site/days/{DAY}.json"));
    until("leader closes the first day", || closed.is_file()).await;

    let mut follower = peer_config(root.path(), "p01", &model, follower_peers, shared);
    follower.last_day = Some(shift(day(), 1));
    let (stop_follower, follower_rx) = watch::channel(false);
    let follower_run = tokio::spawn(async move { start(follower, follower_rx).await });
    let copied = root.path().join(format!("p01-site/days/{DAY}.json"));
    until("follower recomputes yesterday", || copied.is_file()).await;
    assert_eq!(
        std::fs::read(&copied).unwrap(),
        std::fs::read(&closed).unwrap()
    );

    let leader_status = root.path().join("p00-site/status.json");
    let follower_status = root.path().join("p01-site/status.json");
    until("both peers paint yesterday", || {
        let leader = read_json(&leader_status);
        let follower = read_json(&follower_status);
        leader["previous_day"]["day"] == DAY
            && follower["previous_day"] == leader["previous_day"]
            && follower["previous_day"]["paint"] != "no_record"
    })
    .await;
    let archive = std::fs::read_to_string(root.path().join("p01-site/days/index.html")).unwrap();
    assert!(archive.contains(&format!("href=\"{DAY}.html\"")));
    assert!(!root.path().join("p01-site/days/2026-10-25.json").is_file());

    drop(stop_leader);
    drop(stop_follower);
    leader_run.await.unwrap().unwrap();
    follower_run.await.unwrap().unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_partial_closed_day_is_not_painted_until_catch_up() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let mut leader = peer_config(root.path(), "p00", &model, Vec::new(), clock_at(1));
    leader.activation_interval_ms = plumage::config::MAX_ACTIVATION_INTERVAL_MS;
    seed_votes(&leader, day(), CATCH_UP_PAGES * 16 + 1).await;
    let shared = clock_at(4_000);
    leader.clock = shared;
    leader.last_day = None;

    let (stop_leader, leader_rx) = watch::channel(false);
    let leader_run = tokio::spawn(async move { start(leader, leader_rx).await });
    let closed = root.path().join(format!("p00-site/days/{DAY}.json"));
    until("leader closes the seeded day", || closed.is_file()).await;

    let mut follower = peer_config(
        root.path(),
        "p01",
        &model,
        vec![StaticPeer {
            name: "p00".to_owned(),
            address: Ipv6Addr::LOCALHOST,
            advertisement_file: root.path().join("p00.ad"),
        }],
        shared,
    );
    follower.last_day = None;
    follower.sync_interval_ms = 8_000;
    follower.activation_interval_ms = plumage::config::MAX_ACTIVATION_INTERVAL_MS;
    let (stop_follower, follower_rx) = watch::channel(false);
    let follower_run = tokio::spawn(async move { start(follower, follower_rx).await });

    let redb = root.path().join(format!("p01/{DAY}.redb"));
    let started = tokio::time::Instant::now();
    let mut imported = false;
    while started.elapsed() < Duration::from_secs(30) {
        if std::fs::metadata(&redb).map(|meta| meta.len()).unwrap_or(0) > 100_000 {
            imported = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(imported, "the first tick did not import a partial ledger");
    let copied = root.path().join(format!("p01-site/days/{DAY}.json"));
    assert!(!copied.is_file(), "a partial day was painted");
    let status = read_json(&root.path().join("p01-site/status.json"));
    assert_eq!(status["previous_day"]["paint"], "no_record");

    until("catch-up rewrites the closed day", || copied.is_file()).await;
    assert_eq!(
        std::fs::read(&copied).unwrap(),
        std::fs::read(&closed).unwrap()
    );

    drop(stop_leader);
    drop(stop_follower);
    leader_run.await.unwrap().unwrap();
    follower_run.await.unwrap().unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_days_older_than_seven_leave_the_sync_and_stay_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let mut config = peer_config(root.path(), "p00", &model, Vec::new(), clock_at(50_000));
    config.last_day = None;
    config.activation_interval_ms = plumage::config::MAX_ACTIVATION_INTERVAL_MS;
    let (stop, shutdown) = watch::channel(false);
    let running = tokio::spawn(async move { start(config, shutdown).await });

    let held = shift(day(), 8);
    let status_path = root.path().join("p00-site/status.json");
    until("the peer reaches the eighth day ahead", || {
        status_path.is_file() && read_json(&status_path)["day"] == held.to_string()
    })
    .await;

    let ad = root.path().join("p00.ad");
    until("the advertisement is published", || ad.is_file()).await;
    let (gone, body) = post_exchange(&ad, &day().to_string()).await;
    assert_eq!(gone, 409, "{body}");
    assert_eq!(body["code"], "conflict");
    let (kept, kept_body) = post_exchange(&ad, &held.to_string()).await;
    assert_eq!(kept, 200, "{kept_body}");
    assert!(root.path().join(format!("p00/{DAY}.redb")).is_file());
    assert!(
        root.path()
            .join(format!("p00-site/days/{DAY}.json"))
            .is_file()
    );

    drop(stop);
    running.await.unwrap().unwrap();
    server.abort();
}
