//! Offline Liskov mode: config, directory source, loopback exchange, the
//! peer process, the site server and `plumage launch`.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plumage::config::PeerConfig;
use plumage::day::Day;
use plumage::fetch::{FetchError, Source};
use plumage::liskov::{
    self, DirectoryPeer, RuntimeEnv, missing_target, one_target, runtime_peer_id,
};
use plumage::peer::{self, PeerSource};
use plumage::serve;
use serde_json::{Value, json};
use tokio::sync::watch;

#[path = "support/mod.rs"]
mod support;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const NOW: u64 = 1_790_000_001_000;

fn payload() -> Value {
    json!({
        "data_dir": "state/data",
        "site_dir": "state/site",
        "model_url": "http://127.0.0.1:8081/",
        "listen": "[::]:8841",
        "clock": {"real_start_ms": 0, "sim_start_ms": 0, "speed": 1},
        "last_day": null,
        "liskov": {"site_listen": "127.0.0.1:8840"}
    })
}

fn parse(value: Value) -> PeerConfig {
    serde_json::from_value(value).unwrap()
}

fn env(root: &Path) -> RuntimeEnv {
    RuntimeEnv {
        runtime_instance_id: "instance-a".to_owned(),
        advertisement_file: root.join("advert.json"),
        directory_file: root.join("directory.json"),
    }
}

#[test]
fn payload_shape_applies_and_rejects_a_published_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut config = parse(payload());
    liskov::apply(&mut config, &env(root.path()), TOKEN.to_owned()).unwrap();
    config.validate().unwrap();
    assert_eq!(config.peer_id, runtime_peer_id("instance-a"));
    assert_eq!(config.advertisement_file, root.path().join("advert.json"));
    assert_eq!(config.token, TOKEN);

    for key in ["peer_id", "token", "advertisement_file"] {
        let mut filed = payload();
        filed[key] = json!("0123456789abcdef0123456789abcdef");
        let mut config = parse(filed);
        let error = liskov::apply(&mut config, &env(root.path()), TOKEN.to_owned()).unwrap_err();
        assert_eq!(error.code, "config", "{key}");
    }
    let mut peers = payload();
    peers["peers"] = json!([{
        "name": "p00",
        "address": "::1",
        "advertisement_file": "p00.ad"
    }]);
    let mut config = parse(peers);
    assert_eq!(
        liskov::apply(&mut config, &env(root.path()), TOKEN.to_owned())
            .unwrap_err()
            .code,
        "config"
    );

    let mut config = parse(payload());
    liskov::apply(&mut config, &env(root.path()), "x".repeat(31)).unwrap();
    assert_eq!(config.validate().unwrap_err().code, "config");

    let mut remote = payload();
    remote["liskov"]["site_listen"] = json!("192.0.2.1:8840");
    assert_eq!(parse(remote).validate().unwrap_err().code, "config");
    let mut portless = payload();
    portless["liskov"]["site_listen"] = json!("127.0.0.1:0");
    assert_eq!(parse(portless).validate().unwrap_err().code, "config");
}

fn peers_at(
    path: &Path,
    now: &Arc<AtomicU64>,
    lines: &Arc<Mutex<Vec<String>>>,
) -> liskov::LiskovPeers {
    let clock = Arc::clone(now);
    let log = Arc::clone(lines);
    liskov::LiskovPeers::with(
        RuntimeEnv {
            runtime_instance_id: "instance-a".to_owned(),
            advertisement_file: path.with_file_name("advert.json"),
            directory_file: path.to_owned(),
        },
        move || clock.load(Ordering::Relaxed),
        move |line| log.lock().unwrap().push(line.to_owned()),
    )
}

#[test]
fn directory_reader_filters_service_expiry_and_rereads_once_a_second() {
    let root = tempfile::tempdir().unwrap();
    let fixture = include_str!("fixtures/liskov-discovery-v2-directory.json");
    let path = root.path().join("directory.json");
    std::fs::write(&path, fixture).unwrap();
    let now = Arc::new(AtomicU64::new(NOW));
    let lines = Arc::new(Mutex::new(Vec::new()));
    let peers = peers_at(&path, &now, &lines);

    let found = peers.read();
    let target = found[&runtime_peer_id("instance-b")]
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(target.name, runtime_peer_id("instance-b"));
    assert_eq!(target.url, "https://[2606:4700:4700::1111]:8841/");
    assert!(target.tls_fingerprint.starts_with("sha256:"));

    // A later empty directory. A read would drop the peer. Inside the second it must not.
    std::fs::write(
        &path,
        r#"{"schema":"liskov.discovery.v2","applicationUid":"app-0123456789abcdef0123456789abcdef","self":{"jobId":"job-a","runtimeInstanceId":"instance-a"},"revision":"sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945","observedAtMs":1790000003000,"validUntilMs":1790000080000,"peers":[]}"#,
    )
    .unwrap();
    now.store(NOW + 500, Ordering::Relaxed);
    assert_eq!(
        peers.read().len(),
        1,
        "a second read inside one second must reuse the file"
    );

    now.store(NOW + 1_000, Ordering::Relaxed);
    assert!(
        peers.read().is_empty(),
        "the next second reads the emptied file"
    );

    let expired_now = Arc::new(AtomicU64::new(1_790_000_080_000));
    let expired_lines = Arc::new(Mutex::new(Vec::new()));
    std::fs::write(&path, fixture).unwrap();
    let expired = peers_at(&path, &expired_now, &expired_lines);
    assert!(
        expired.read().is_empty(),
        "an expired directory gives no targets"
    );

    let swarm_now = Arc::new(AtomicU64::new(NOW));
    let swarm_lines = Arc::new(Mutex::new(Vec::new()));
    std::fs::write(
        &path,
        include_str!("fixtures/liskov-discovery-v2-swarm.json"),
    )
    .unwrap();
    let swarm = peers_at(&path, &swarm_now, &swarm_lines);
    assert!(swarm.read().is_empty());
    let logged = swarm_lines.lock().unwrap().clone();
    assert!(
        logged.iter().any(|line| {
            line == "plumage directory outcome=unavailable peers=0 error=discovery_invalid"
        }),
        "{logged:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_peers_exchange_through_the_directory_mapping() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let left_id = runtime_peer_id("instance-a");
    let right_id = runtime_peer_id("instance-b");
    let left_ad = root.path().join("left.ad");
    let right_ad = root.path().join("right.ad");
    let left = lab_peer(root.path(), &left_id, &model, &left_ad);
    let right = lab_peer(root.path(), &right_id, &model, &right_ad);
    let left_source = mapped(right_id.clone(), right_ad);
    let right_source = mapped(left_id.clone(), left_ad);
    let (_left_stop, left_rx) = watch::channel(false);
    let (_right_stop, right_rx) = watch::channel(false);
    let run = tokio::time::timeout(Duration::from_secs(60), async {
        let (left_run, right_run) = tokio::join!(
            peer::run(left, EmptySource, left_source, left_rx),
            peer::run(right, EmptySource, right_source, right_rx),
        );
        left_run.unwrap();
        right_run.unwrap();
    });
    run.await.expect("exchange finished within 60s");
    server.abort();

    let mut ids = vec![left_id, right_id];
    ids.sort();
    for id in &ids {
        let status: Value = serde_json::from_slice(
            &std::fs::read(root.path().join(format!("{id}-site/status.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(status["authors_seen"], json!(ids));
    }
}

fn mapped(peer_id: String, advertisement: std::path::PathBuf) -> PeerSource {
    PeerSource::mapped(move || {
        let Ok(bytes) = std::fs::read(&advertisement) else {
            return missing_target(&peer_id);
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return missing_target(&peer_id);
        };
        if value.get("ready") != Some(&json!(true)) {
            return missing_target(&peer_id);
        }
        let Some(port) = value.get("port").and_then(Value::as_u64) else {
            return missing_target(&peer_id);
        };
        let Some(pin) = value.get("tlsFingerprint").and_then(Value::as_str) else {
            return missing_target(&peer_id);
        };
        let peer = DirectoryPeer {
            job_ref: "job-test".to_owned(),
            runtime_instance_id: "instance".to_owned(),
            peer_id: peer_id.clone(),
            url: format!("https://[::1]:{port}/"),
            tls_fingerprint: pin.to_owned(),
            valid_until_ms: u64::MAX,
        };
        // `one_target` applies `directory_target`, including this loopback URL.
        one_target(&peer)
    })
}

fn lab_peer(root: &Path, id: &str, model_url: &str, advertisement: &Path) -> PeerConfig {
    let day = Day::from_str("2026-10-26").unwrap();
    PeerConfig {
        peer_id: id.to_owned(),
        data_dir: root.join(id),
        site_dir: root.join(format!("{id}-site")),
        model_url: model_url.to_owned(),
        listen: "[::1]:0".parse().unwrap(),
        advertisement_file: advertisement.to_owned(),
        peers: Vec::new(),
        token: TOKEN.to_owned(),
        clock: plumage::clock::DayClock {
            real_start_ms: wall_now(),
            sim_start_ms: day.open_ms(),
            speed: 2880,
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
    }
}

fn wall_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn liskov_process_serves_status_then_exits_on_term() {
    let root = tempfile::tempdir().unwrap();
    let (model, server) = support::spawn().await;
    let site_port = free_port("127.0.0.1:0");
    let sync_port = free_port("[::1]:0");
    let config_path = root.path().join("peer.json");
    let ready = root.path().join("ready");
    let advert = root.path().join("advert.json");
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/liskov-discovery-v2-directory.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "data_dir": root.path().join("data"),
            "site_dir": root.path().join("site"),
            "model_url": model,
            "listen": format!("[::1]:{sync_port}"),
            "clock": {"real_start_ms": 0, "sim_start_ms": 0, "speed": 1},
            "last_day": null,
            "fetch_timeout_seconds": 1,
            "request_timeout_seconds": 1,
            "inference_timeout_seconds": 5,
            "liskov": {"site_listen": format!("127.0.0.1:{site_port}")}
        }))
        .unwrap(),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_plumage"))
        .arg("peer")
        .arg(&config_path)
        .env("LISKOV_RUNTIME_INSTANCE_ID", "instance-a")
        .env("LISKOV_PEER_ADVERTISEMENT_FILE", &advert)
        .env("LISKOV_PEER_DIRECTORY_FILE", &directory)
        .env("LISKOV_PLUMAGE_PEER_TOKEN", TOKEN)
        .env("LISKOV_APPLICATION_READY_FILE", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let status_url = format!("http://127.0.0.1:{site_port}/status.json");
    let started = Instant::now();
    let mut saw_status = false;
    loop {
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "peer did not become ready\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let status_ok = client
            .get(&status_url)
            .send()
            .await
            .ok()
            .is_some_and(|response| response.status().as_u16() == 200);
        if ready.is_file() && !status_ok && !saw_status {
            let _ = child.kill();
            panic!("ready file existed before status.json answered 200");
        }
        if status_ok {
            saw_status = true;
        }
        if ready.is_file() && saw_status && advert.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let body: Value = client
        .get(&status_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["peer_id"], runtime_peer_id("instance-a"));
    assert!(advert.is_file());

    let killed = Instant::now();
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    loop {
        if let Some(code) = child.try_wait().unwrap() {
            assert!(code.success(), "stderr after TERM: {code}");
            assert!(killed.elapsed() < Duration::from_secs(10));
            break;
        }
        if killed.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("peer did not exit within 10s of TERM");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.abort();
}

#[test]
fn missing_peer_token_names_the_variable_and_launch_names_the_payload() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("peer.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(&json!({
            "data_dir": root.path().join("data"),
            "site_dir": root.path().join("site"),
            "model_url": "http://127.0.0.1:9/",
            "listen": "[::1]:9",
            "clock": {"real_start_ms": 0, "sim_start_ms": 0, "speed": 1},
            "last_day": null,
            "liskov": {"site_listen": "127.0.0.1:9"}
        }))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_plumage"))
        .args(["peer", config_path.to_str().unwrap()])
        .env("LISKOV_RUNTIME_INSTANCE_ID", "instance-a")
        .env(
            "LISKOV_PEER_ADVERTISEMENT_FILE",
            root.path().join("advert.json"),
        )
        .env(
            "LISKOV_PEER_DIRECTORY_FILE",
            root.path().join("directory.json"),
        )
        .env_remove("LISKOV_PLUMAGE_PEER_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("LISKOV_PLUMAGE_PEER_TOKEN"), "{stderr}");

    let launch = Command::new(env!("CARGO_BIN_EXE_plumage"))
        .arg("launch")
        .env_remove("LISKOV_PLUMAGE_PAYLOAD_URL")
        .output()
        .unwrap();
    assert_eq!(launch.status.code(), Some(1));
    let stderr = String::from_utf8(launch.stderr).unwrap();
    assert!(stderr.contains("missing payload variable"), "{stderr}");

    let usage = Command::new(env!("CARGO_BIN_EXE_plumage"))
        .output()
        .unwrap();
    assert!(usage.status.success());
    let text = String::from_utf8(usage.stdout).unwrap();
    assert!(text.contains("plumage launch"), "{text}");
}

#[tokio::test]
async fn serve_is_read_only_and_refuses_escape() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("index.html"), b"<p>front</p>").unwrap();
    std::fs::write(root.path().join("status.json"), b"{\"ok\":true}").unwrap();
    std::fs::write(root.path().join("note.txt"), b"note").unwrap();
    std::fs::create_dir(root.path().join("days")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, mut stop_rx) = watch::channel(false);
    let dir = root.path().to_owned();
    let task = tokio::spawn(async move {
        serve::run(dir, listener, async move {
            loop {
                if *stop_rx.borrow() {
                    return;
                }
                if stop_rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let front = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(front.status(), 200);
    assert_eq!(front.text().await.unwrap(), "<p>front</p>");
    let status = client
        .get(format!("{base}/status.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(status.headers().get("cache-control").unwrap(), "no-store");
    let head = client
        .head(format!("{base}/note.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert!(head.bytes().await.unwrap().is_empty());
    let post = client
        .post(format!("{base}/note.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), 405);
    for path in ["/../Cargo.toml", "/%2e%2e/x", "/days/", "/missing.txt"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
    let _ = stop.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap();
}

fn free_port(address: &str) -> u16 {
    let listener = TcpListener::bind(address).unwrap();
    listener.local_addr().unwrap().port()
}

struct EmptySource;

impl Source for EmptySource {
    fn get(
        &self,
        url: &url::Url,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, FetchError>> + Send {
        let url = url.clone();
        async move {
            Err(FetchError {
                code: "fetch_failed",
                message: format!("test source refuses {url}"),
            })
        }
    }
}
