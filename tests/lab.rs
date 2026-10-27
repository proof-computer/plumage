//! Three local peers, one compressed day, against the stateless fake model.

use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[path = "support/mod.rs"]
mod support;

fn plumage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_plumage"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_config(args: &[&str]) {
    let output = plumage(args);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.starts_with("config:"), "{stderr}");
}

#[test]
fn lab_refuses_a_non_empty_dir_and_out_of_range_flags() {
    let occupied = tempfile::tempdir().unwrap();
    std::fs::write(occupied.path().join("kept"), b"x").unwrap();
    let occupied = occupied.path().to_str().unwrap();
    assert_config(&["lab", "--dir", occupied]);

    let empty = tempfile::tempdir().unwrap();
    let empty = empty.path().to_str().unwrap();
    assert_config(&["lab", "--dir", empty, "--peers", "0"]);
    assert_config(&["lab", "--dir", empty, "--peers", "65"]);
    assert_config(&["lab", "--dir", empty, "--speed", "0"]);
    assert_config(&["lab", "--dir", empty, "--no-such-flag"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_peers_agree_on_blue_through_one_compressed_day() {
    let finished = tokio::time::timeout(Duration::from_secs(120), three_peers()).await;
    finished.expect("lab bounded at 120s");
}

async fn three_peers() {
    let (model, server) = support::spawn_lab().await;
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_plumage"))
        .arg("lab")
        .arg("--dir")
        .arg(dir.path())
        .arg("--peers")
        .arg("3")
        .arg("--model-url")
        .arg(&model)
        .arg("--day")
        .arg("2026-10-26")
        .arg("--days")
        .arg("1")
        .arg("--speed")
        .arg("2880")
        .arg("--activation-interval-ms")
        .arg("1800000")
        .arg("--sync-interval-ms")
        .arg("100")
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    let elapsed = started.elapsed();
    server.abort();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        output.status.success(),
        "elapsed={elapsed:?} stdout={stdout} stderr={stderr}"
    );
    // Speed 2880 is 30s of wall time for the day, plus peer startup.
    assert!(
        elapsed < Duration::from_secs(50),
        "expected about 40s, took {elapsed:?}"
    );
    assert!(stdout.contains("agree=true"), "{stdout}");
    let index = dir.path().join("p00/site/index.html");
    assert!(stdout.contains(&index.display().to_string()), "{stdout}");

    let mut documents = Vec::new();
    for name in ["p00", "p01", "p02"] {
        let path = dir.path().join(format!("{name}/site/days/2026-10-26.json"));
        let bytes = std::fs::read(&path).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["decision"]["colour"], "blue", "{name}");
        assert_eq!(document["live_voters"], json!(["p00", "p01", "p02"]));
        assert!(document["decision"]["held_from_ms"].is_u64());
        documents.push(bytes);
    }
    assert_eq!(documents[0], documents[1]);
    assert_eq!(documents[1], documents[2]);

    let index_html = std::fs::read_to_string(&index).unwrap();
    assert!(index_html.contains("background:#0000FF"), "{index_html}");
    let day_html =
        std::fs::read_to_string(dir.path().join("p00/site/days/2026-10-26.html")).unwrap();
    assert!(
        day_html.contains("argument from p02"),
        "p02's argument did not reach p00"
    );

    let lab: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("lab.json")).unwrap()).unwrap();
    assert!(lab.get("token").is_none());
    assert_eq!(lab["peers"], json!(["p00", "p01", "p02"]));
    assert_eq!(lab["first_day"], "2026-10-26");
    assert_eq!(lab["speed"], 2880);
    let config: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("p00/config.json")).unwrap())
            .unwrap();
    assert_eq!(config["listen"], "[::1]:0");
    let others = config["peers"].as_array().unwrap();
    assert_eq!(others.len(), 2);
    assert!(others.iter().all(|peer| peer["address"] == "::1"));
    assert!(others.iter().all(|peer| peer["name"] != "p00"));
}
