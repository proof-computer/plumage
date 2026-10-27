//! Launch N local peers on loopback against one model server.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use forthebirds_net::StaticPeer;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::clock::DayClock;
use crate::config::{
    MAX_ACTIVATION_INTERVAL_MS, MAX_SYNC_INTERVAL_MS, MIN_ACTIVATION_INTERVAL_MS,
    MIN_SYNC_INTERVAL_MS, PeerConfig,
};
use crate::day::Day;
use crate::error::{Error, Result};

const DEFAULT_PEERS: usize = 12;
const DEFAULT_MODEL_URL: &str = "http://127.0.0.1:8080/";
const MIN_PEERS: usize = 1;
const MAX_PEERS: usize = 64;
const MIN_DAYS: u32 = 1;
const MAX_DAYS: u32 = 366;
const MIN_SPEED: u64 = 1;
const MAX_SPEED: u64 = 100_000;

/// What `plumage lab` was asked to run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabConfig {
    pub dir: PathBuf,
    pub peers: usize,
    pub model_url: String,
    pub first_day: Day,
    pub days: u32,
    pub speed: u64,
    pub activation_interval_ms: Option<u64>,
    pub sync_interval_ms: Option<u64>,
}

/// One peer's decision for one day, read from `site/days/<D>.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerDecision {
    pub peer: String,
    pub decision: Value,
}

/// One simulated day across the peers that finished it.
#[derive(Clone, Debug, PartialEq)]
pub struct DayReport {
    pub day: Day,
    pub peers: Vec<PeerDecision>,
    pub agree: bool,
}

/// The days the lab waited for, and `p00`'s front page.
#[derive(Clone, Debug, PartialEq)]
pub struct LabReport {
    pub days: Vec<DayReport>,
    pub index: PathBuf,
}

impl std::fmt::Display for LabReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for day in &self.days {
            write!(formatter, "{}", day.day)?;
            for peer in &day.peers {
                write!(
                    formatter,
                    " {}={}",
                    peer.peer,
                    decision_label(&peer.decision)
                )?;
            }
            writeln!(formatter, " agree={}", day.agree)?;
        }
        writeln!(formatter, "{}", self.index.display())
    }
}

/// `--dir` is required. Anything outside the flags below is `config`.
pub fn parse(args: &[String]) -> Result<LabConfig> {
    let mut dir = None;
    let mut peers = DEFAULT_PEERS;
    let mut model_url = DEFAULT_MODEL_URL.to_owned();
    let mut day = None;
    let mut days = 1u32;
    let mut speed = 1u64;
    let mut activation_interval_ms = None;
    let mut sync_interval_ms = None;
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--dir" => dir = Some(PathBuf::from(value(&mut rest, flag)?)),
            "--peers" => {
                peers = usize::try_from(bounded(
                    value(&mut rest, flag)?,
                    flag,
                    u64::try_from(MIN_PEERS).expect("min peers"),
                    u64::try_from(MAX_PEERS).expect("max peers"),
                )?)
                .expect("peer count fits usize");
            }
            "--model-url" => model_url = value(&mut rest, flag)?.to_owned(),
            "--day" => {
                day = Some(
                    value(&mut rest, flag)?
                        .parse()
                        .map_err(|_| Error::new("config", "day must be YYYY-MM-DD"))?,
                );
            }
            "--days" => {
                days = u32::try_from(bounded(
                    value(&mut rest, flag)?,
                    flag,
                    u64::from(MIN_DAYS),
                    u64::from(MAX_DAYS),
                )?)
                .expect("day count fits u32");
            }
            "--speed" => {
                speed = bounded(value(&mut rest, flag)?, flag, MIN_SPEED, MAX_SPEED)?;
            }
            "--activation-interval-ms" => {
                activation_interval_ms = Some(bounded(
                    value(&mut rest, flag)?,
                    flag,
                    MIN_ACTIVATION_INTERVAL_MS,
                    MAX_ACTIVATION_INTERVAL_MS,
                )?);
            }
            "--sync-interval-ms" => {
                sync_interval_ms = Some(bounded(
                    value(&mut rest, flag)?,
                    flag,
                    MIN_SYNC_INTERVAL_MS,
                    MAX_SYNC_INTERVAL_MS,
                )?);
            }
            other => {
                return Err(Error::new("config", format!("unknown argument {other}")));
            }
        }
    }
    let Some(dir) = dir else {
        return Err(Error::new("config", "missing --dir"));
    };
    if model_url.is_empty() {
        return Err(Error::new("config", "model url is empty"));
    }
    Ok(LabConfig {
        dir,
        peers,
        model_url,
        first_day: day.unwrap_or_else(|| Day::from_ms(forthebirds_store::now())),
        days,
        speed,
        activation_interval_ms,
        sync_interval_ms,
    })
}

/// Write one configuration per peer, launch `plumage peer`, and wait.
///
/// A `dir` that exists and is not empty is `config`. `shutdown` kills the
/// children and returns `interrupted`. A peer that exits non-zero, or that
/// writes no result for a requested day, is `peer_failed`.
pub async fn run(config: LabConfig, mut shutdown: watch::Receiver<bool>) -> Result<LabReport> {
    if *shutdown.borrow() {
        return Err(Error::new("interrupted", "shutdown"));
    }
    prepare_dir(&config.dir)?;
    let names = peer_names(config.peers);
    let token = read_token()?;
    let clock = DayClock {
        real_start_ms: forthebirds_store::now(),
        sim_start_ms: config.first_day.open_ms(),
        speed: config.speed,
    };
    let last = shift_days(config.first_day, config.days - 1);
    write_peer_configs(&config, &names, &token, clock, last)?;
    write_lab_file(&config, &names, clock.real_start_ms)?;
    let mut children = spawn_peers(&config.dir, &names)?;
    wait_all(&mut children, &mut shutdown).await?;
    collect(&config, &names, &children)
}

fn value<'a>(args: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<&'a str> {
    match args.next() {
        Some(text) if !text.starts_with("--") => Ok(text),
        _ => Err(Error::new("config", format!("missing value for {flag}"))),
    }
}

fn bounded(text: &str, flag: &str, min: u64, max: u64) -> Result<u64> {
    let value: u64 = text
        .parse()
        .map_err(|_| Error::new("config", format!("{flag} must be from {min} to {max}")))?;
    if !(min..=max).contains(&value) {
        return Err(Error::new(
            "config",
            format!("{flag} must be from {min} to {max}"),
        ));
    }
    Ok(value)
}

fn peer_names(peers: usize) -> Vec<String> {
    (0..peers).map(|index| format!("p{index:02}")).collect()
}

fn shift_days(mut day: Day, by: u32) -> Day {
    for _ in 0..by {
        day = day.next();
    }
    day
}

fn prepare_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(Error::new("config", "dir exists and is not a directory"));
        }
        let mut entries = fs::read_dir(dir)
            .map_err(|error| Error::new("config", format!("dir cannot be read: {error}")))?;
        if entries.next().is_some() {
            return Err(Error::new("config", "dir exists and is not empty"));
        }
    }
    fs::create_dir_all(dir).map_err(|error| Error::new("config", format!("dir: {error}")))
}

fn read_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    let mut file = File::open("/dev/urandom")
        .map_err(|error| Error::new("config", format!("run token: {error}")))?;
    file.read_exact(&mut bytes)
        .map_err(|error| Error::new("config", format!("run token: {error}")))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push_str(&format!("{byte:02x}"));
    }
    Ok(token)
}

fn write_peer_configs(
    config: &LabConfig,
    names: &[String],
    token: &str,
    clock: DayClock,
    last_day: Day,
) -> Result<()> {
    for (index, name) in names.iter().enumerate() {
        let peer_dir = config.dir.join(name);
        let data_dir = peer_dir.join("data");
        let site_dir = peer_dir.join("site");
        fs::create_dir_all(&data_dir)
            .map_err(|error| Error::new("config", format!("{name} data: {error}")))?;
        fs::create_dir_all(&site_dir)
            .map_err(|error| Error::new("config", format!("{name} site: {error}")))?;
        let advertisement = peer_dir.join("advert.json");
        let peers: Vec<StaticPeer> = names
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, other)| StaticPeer {
                name: other.clone(),
                address: Ipv6Addr::LOCALHOST,
                advertisement_file: config.dir.join(other).join("advert.json"),
            })
            .collect();
        let mut written = json!({
            "peer_id": name,
            "data_dir": data_dir,
            "site_dir": site_dir,
            "model_url": config.model_url,
            "listen": "[::1]:0",
            "advertisement_file": advertisement,
            "peers": peers,
            "token": token,
            "clock": clock,
            "last_day": last_day,
        });
        if let Some(interval) = config.activation_interval_ms {
            written["activation_interval_ms"] = json!(interval);
        }
        if let Some(interval) = config.sync_interval_ms {
            written["sync_interval_ms"] = json!(interval);
        }
        let text = serde_json::to_string_pretty(&written)
            .map_err(|error| Error::new("config", error.to_string()))?;
        let parsed: PeerConfig =
            serde_json::from_str(&text).map_err(|error| Error::new("config", error.to_string()))?;
        parsed.validate()?;
        let path = peer_dir.join("config.json");
        fs::write(&path, text + "\n")
            .map_err(|error| Error::new("config", format!("{}: {error}", path.display())))?;
    }
    Ok(())
}

fn write_lab_file(config: &LabConfig, names: &[String], start_time: u64) -> Result<()> {
    #[derive(Serialize)]
    struct LabFile<'a> {
        peers: &'a [String],
        first_day: Day,
        days: u32,
        speed: u64,
        model_url: &'a str,
        start_time: u64,
    }
    let file = LabFile {
        peers: names,
        first_day: config.first_day,
        days: config.days,
        speed: config.speed,
        model_url: &config.model_url,
        start_time,
    };
    let mut text = serde_json::to_string_pretty(&file)
        .map_err(|error| Error::new("config", error.to_string()))?;
    text.push('\n');
    let path = config.dir.join("lab.json");
    fs::write(&path, text)
        .map_err(|error| Error::new("config", format!("{}: {error}", path.display())))
}

struct Running {
    name: String,
    child: Child,
    log: PathBuf,
    status: Option<std::process::ExitStatus>,
}

fn spawn_peers(dir: &Path, names: &[String]) -> Result<Vec<Running>> {
    let exe = std::env::current_exe()
        .map_err(|error| Error::new("peer_failed", format!("plumage binary: {error}")))?;
    let mut running = Vec::with_capacity(names.len());
    for name in names {
        let peer_dir = dir.join(name);
        let log_path = peer_dir.join("peer.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| {
                Error::new(
                    "peer_failed",
                    format!("{name} log {}: {error}", log_path.display()),
                )
            })?;
        let stderr = log.try_clone().map_err(|error| {
            Error::new(
                "peer_failed",
                format!("{name} log {}: {error}", log_path.display()),
            )
        })?;
        let child = Command::new(&exe)
            .arg("peer")
            .arg(peer_dir.join("config.json"))
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                Error::new(
                    "peer_failed",
                    format!("{name} spawn failed: {error}; log {}", log_path.display()),
                )
            })?;
        running.push(Running {
            name: name.clone(),
            child,
            log: log_path,
            status: None,
        });
    }
    Ok(running)
}

async fn wait_all(children: &mut [Running], shutdown: &mut watch::Receiver<bool>) -> Result<()> {
    loop {
        if *shutdown.borrow() {
            kill_all(children);
            return Err(Error::new("interrupted", "shutdown"));
        }
        let mut pending = false;
        let mut wait_error = None;
        for peer in children.iter_mut() {
            if peer.status.is_some() {
                continue;
            }
            match peer.child.try_wait() {
                Ok(Some(status)) => peer.status = Some(status),
                Ok(None) => pending = true,
                Err(error) => {
                    wait_error = Some(format!(
                        "{} wait failed: {error}; log {}",
                        peer.name,
                        peer.log.display()
                    ));
                    break;
                }
            }
        }
        if let Some(message) = wait_error {
            kill_all(children);
            return Err(Error::new("peer_failed", message));
        }
        if !pending {
            return Ok(());
        }
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    kill_all(children);
                    return Err(Error::new("interrupted", "shutdown"));
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

fn kill_all(children: &mut [Running]) {
    for peer in children {
        let _ = peer.child.start_kill();
    }
}

fn collect(config: &LabConfig, names: &[String], children: &[Running]) -> Result<LabReport> {
    let mut failures = Vec::new();
    for peer in children {
        let status = peer.status.expect("wait_all records every status");
        if !status.success() {
            let code = status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned());
            failures.push(format!(
                "{} exited {code}; log {}",
                peer.name,
                peer.log.display()
            ));
        }
    }
    if !failures.is_empty() {
        return Err(Error::new("peer_failed", failures.join("; ")));
    }
    let mut days = Vec::with_capacity(usize::try_from(config.days).unwrap_or(0));
    let mut day = config.first_day;
    for index in 0..config.days {
        if index > 0 {
            day = day.next();
        }
        let mut peers = Vec::with_capacity(names.len());
        for name in names {
            let path = config.dir.join(name).join(format!("site/days/{day}.json"));
            let decision = read_decision(&path).map_err(|error| {
                Error::new(
                    "peer_failed",
                    format!(
                        "{name} wrote no result {}: {error}; log {}",
                        path.display(),
                        config.dir.join(name).join("peer.log").display()
                    ),
                )
            })?;
            peers.push(PeerDecision {
                peer: name.clone(),
                decision,
            });
        }
        let agree = peers
            .first()
            .is_some_and(|first| peers.iter().all(|peer| peer.decision == first.decision));
        days.push(DayReport { day, peers, agree });
    }
    Ok(LabReport {
        days,
        index: config.dir.join("p00/site/index.html"),
    })
}

fn read_decision(path: &Path) -> std::result::Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    value
        .get("decision")
        .cloned()
        .ok_or_else(|| "decision is missing".to_owned())
}

fn decision_label(decision: &Value) -> String {
    if let Some(colour) = decision.get("colour").and_then(Value::as_str) {
        return colour.to_owned();
    }
    if let Some(text) = decision.as_str() {
        return text.to_owned();
    }
    decision.to_string()
}
