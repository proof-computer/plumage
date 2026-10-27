//! A whole simulated day, in process, against the stateless fake model.

use std::future::Future;
use std::net::Ipv6Addr;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use forthebirds_net::StaticPeer;
use plumage::clock::DayClock;
use plumage::config::PeerConfig;
use plumage::day::Day;
use plumage::fetch::{FetchError, Source};
use plumage::peer;
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
    DayClock {
        real_start_ms: wall_now(),
        sim_start_ms: day().open_ms(),
        speed: 2880,
    }
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
    }
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
    peer::run(config, EmptySource, receiver).await.unwrap();
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
    let (left_run, right_run) = tokio::join!(
        peer::run(left, EmptySource, left_rx),
        peer::run(right, EmptySource, right_rx),
    );
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
