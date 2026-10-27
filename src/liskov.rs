//! Liskov-mode peer: fill the runtime identity, follow the helper directory,
//! serve the site, and report readiness. Nothing here restarts the process.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use forthebirds_net::PeerTarget;
use liskov_cargo_harness::HarnessApp;
use liskov_cargo_harness::cloudflared::{Prepared, Status};
use liskov_cargo_harness::directory::{self, Directory};
use liskov_cargo_harness::readiness;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::config::PeerConfig;
use crate::error::{Error, Result};
use crate::fetch::HttpSource;
use crate::peer::{self, PeerSource};
use crate::serve;

pub use directory::{DirectoryPeer, RuntimeEnv, runtime_peer_id};

/// One successful target, keyed by its plumage peer id.
pub fn one_target(peer: &DirectoryPeer) -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> {
    let target = directory_target(peer);
    let mut targets = BTreeMap::new();
    targets.insert(target.name.clone(), Ok(target));
    targets
}

/// A not-yet-ready advertisement. The next tick tries again.
pub fn missing_target(name: &str) -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> {
    let mut targets = BTreeMap::new();
    targets.insert(
        name.to_owned(),
        Err(forthebirds_net::Error::new(
            "discovery_unavailable",
            "advertisement file unavailable",
        )),
    );
    targets
}

const SERVICE: &str = "plumage";
const REFRESH_MS: u64 = 1_000;

/// The harness application both `plumage launch` and `plumage peer` use.
///
/// Arguments are `peer <config>`, which is this binary's command. The default
/// descriptor read is unchanged.
pub fn app() -> Result<HarnessApp> {
    let mut app = HarnessApp::new("plumage")?;
    app.peer_args = |path| vec![OsString::from("peer"), path.as_os_str().to_os_string()];
    Ok(app)
}

/// Fill `peer_id`, `advertisement_file` and `token` from the runtime.
///
/// The published file must have left those three empty, and `peers` empty.
/// A credential in the payload is `config`, because two jobs share that file.
pub fn apply(config: &mut PeerConfig, env: &RuntimeEnv, token: String) -> Result<()> {
    if config.liskov.is_none() {
        return Err(Error::new("config", "liskov block is absent"));
    }
    if !config.peer_id.is_empty()
        || !config.token.is_empty()
        || !config.advertisement_file.as_os_str().is_empty()
        || !config.peers.is_empty()
    {
        return Err(Error::new(
            "config",
            "liskov peer.json must leave peer_id, token, advertisement_file and peers empty",
        ));
    }
    config.peer_id = runtime_peer_id(&env.runtime_instance_id);
    config.advertisement_file = env.advertisement_file.clone();
    config.token = token;
    Ok(())
}

/// The same mapping the directory reader uses for each unexpired target.
pub fn directory_target(peer: &DirectoryPeer) -> PeerTarget {
    PeerTarget {
        name: peer.peer_id.clone(),
        url: peer.url.clone(),
        tls_fingerprint: peer.tls_fingerprint.clone(),
    }
}

/// Directory targets, refreshed at most once a second across clones.
///
/// A failed read leaves the previous validity window where it was.
#[derive(Clone)]
pub struct LiskovPeers {
    env: RuntimeEnv,
    cache: Arc<std::sync::Mutex<Cache>>,
    now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

struct Cache {
    directory: Directory,
    last_read_ms: Option<u64>,
    logged_outcome: Option<String>,
    logged_peers: Option<u64>,
}

impl LiskovPeers {
    /// Wall clock, and the directory line on stderr.
    pub fn new(env: RuntimeEnv) -> Self {
        Self::with(env, forthebirds_store::now, |line| eprintln!("{line}"))
    }

    /// `now_ms` is the directory clock. `log` receives one line when the
    /// outcome or the peer count changes. It must not call [`Self::read`].
    pub fn with(
        env: RuntimeEnv,
        now_ms: impl Fn() -> u64 + Send + Sync + 'static,
        log: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            env,
            cache: Arc::new(std::sync::Mutex::new(Cache {
                directory: Directory::default(),
                last_read_ms: None,
                logged_outcome: None,
                logged_peers: None,
            })),
            now_ms: Arc::new(now_ms),
            log: Arc::new(log),
        }
    }

    /// Targets keyed by plumage peer id. The line carries no job reference,
    /// URL or fingerprint.
    pub fn read(&self) -> BTreeMap<String, forthebirds_net::Result<PeerTarget>> {
        let now = (self.now_ms)();
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let due = cache
            .last_read_ms
            .is_none_or(|then| now.saturating_sub(then) >= REFRESH_MS);
        if due {
            let snapshot = directory::read_snapshot(&self.env.directory_file);
            cache
                .directory
                .refresh(snapshot, &self.env.runtime_instance_id, SERVICE, now);
            cache.last_read_ms = Some(now);
        }
        let found = cache.directory.targets(now);
        let status = cache.directory.status(now);
        log_change(&mut cache, &status, &*self.log);
        let mut targets = BTreeMap::new();
        for peer in found.into_values() {
            let target = directory_target(&peer);
            targets.insert(target.name.clone(), Ok(target));
        }
        targets
    }
}

fn log_change(cache: &mut Cache, status: &Value, log: &(dyn Fn(&str) + Send + Sync)) {
    let outcome = status["outcome"]
        .as_str()
        .unwrap_or("unavailable")
        .to_owned();
    let peers = status["peerCount"].as_u64().unwrap_or(0);
    let error = status["error"].as_str().unwrap_or("none").to_owned();
    if cache.logged_outcome.as_deref() == Some(outcome.as_str())
        && cache.logged_peers == Some(peers)
    {
        return;
    }
    cache.logged_outcome = Some(outcome.clone());
    cache.logged_peers = Some(peers);
    log(&format!(
        "plumage directory outcome={outcome} peers={peers} error={error}"
    ));
}

/// Compose the peer, the site server and the optional connector.
///
/// `shutdown` becoming true, or the sender closing, stops every task and
/// returns `Ok`. The peer watches a channel owned here. Cleanup signals that
/// channel, including when the readiness file cannot be written. A peer error
/// is returned after the connector task is joined.
pub async fn run(
    config_path: &Path,
    mut config: PeerConfig,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let app = app()?;
    let fetch = HttpSource::new(Duration::from_secs(config.fetch_timeout_seconds))?;
    if stopped(&shutdown) {
        return Ok(());
    }
    let env = RuntimeEnv::from_env()?;
    let token = peer_token(&app)?;
    apply(&mut config, &env, token)?;
    config.validate()?;

    let prepared = match prepare(&app, &config, config_path, &mut shutdown).await? {
        Prep::Stop => return Ok(()),
        Prep::Ready(prepared) => *prepared,
    };

    let site_listen = config
        .liskov
        .as_ref()
        .ok_or_else(|| Error::new("config", "liskov block is absent"))?
        .site_listen;
    let listener = TcpListener::bind(site_listen)
        .await
        .map_err(|error| Error::new("io", error.to_string()))?;
    let (serve_stop, serve_rx) = watch::channel(false);
    let site_dir = config.site_dir.clone();
    let serve_task = tokio::spawn(async move {
        serve::run(site_dir, listener, async move {
            let mut serve_rx = serve_rx;
            loop {
                if *serve_rx.borrow() {
                    return;
                }
                if serve_rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
    });

    let advertisement = config.advertisement_file.clone();
    let peers = PeerSource::liskov(env);
    // The caller owns `shutdown`. The peer watches this sender so a readiness
    // error, or any other cleanup, can stop it without that sender.
    let (peer_stop, peer_shutdown) = watch::channel(false);
    let bridge = tokio::spawn(forward_shutdown(shutdown.clone(), peer_stop.clone()));
    let peer_task =
        tokio::spawn(async move { peer::run(config, fetch, peers, peer_shutdown).await });

    let state = Arc::new(Mutex::new(Status::new(prepared.is_some())));
    let (conn_stop, conn_rx) = watch::channel(false);
    let conn_task = prepared.map(|prepared| {
        let state = Arc::clone(&state);
        tokio::spawn(async move { prepared.run(state, conn_rx).await })
    });

    let client = reqwest::Client::builder().no_proxy().build();
    let outcome = match client {
        Ok(client) => {
            let status_url = format!("http://{site_listen}/status.json");
            wait_until_stop(
                &app,
                &state,
                &client,
                &status_url,
                &advertisement,
                &mut shutdown,
                &peer_task,
            )
            .await
        }
        Err(error) => Err(Error::new("io", error.to_string())),
    };

    let _ = serve_stop.send(true);
    let _ = conn_stop.send(true);
    let _ = peer_stop.send(true);
    bridge.abort();
    let peer_result = peer_task
        .await
        .unwrap_or_else(|error| Err(Error::new("io", format!("peer task ended: {error}"))));
    if let Some(task) = conn_task {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(error) => return Err(Error::new("io", format!("connector task ended: {error}"))),
        }
    }
    match serve_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(error) => return Err(Error::new("io", format!("site task ended: {error}"))),
    }
    outcome?;
    peer_result
}

enum Prep {
    Stop,
    Ready(Box<Option<Prepared>>),
}

async fn prepare(
    app: &HarnessApp,
    config: &PeerConfig,
    config_path: &Path,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Prep> {
    let Some(liskov) = &config.liskov else {
        return Err(Error::new("config", "liskov block is absent"));
    };
    let Some(mut cloud) = liskov.cloudflared.clone() else {
        return Ok(Prep::Ready(Box::new(None)));
    };
    let root = config_path.parent().unwrap_or(Path::new("."));
    cloud.resolve_paths(root);
    let origin = liskov.site_listen;
    let ports = [origin.port(), config.listen.port()];
    let prepared = tokio::select! {
        biased;
        _ = until_stop(shutdown) => return Ok(Prep::Stop),
        result = Prepared::from_env(app, &cloud, origin, &ports) => result?,
    };
    Ok(Prep::Ready(Box::new(Some(prepared))))
}

async fn wait_until_stop(
    app: &HarnessApp,
    state: &Arc<Mutex<Status>>,
    client: &reqwest::Client,
    status_url: &str,
    advertisement: &Path,
    shutdown: &mut watch::Receiver<bool>,
    peer_task: &JoinHandle<Result<()>>,
) -> Result<()> {
    let mut reported = false;
    loop {
        if stopped(shutdown) || peer_task.is_finished() {
            return Ok(());
        }
        if !reported && advertisement.is_file() && status_ok(client, status_url).await {
            tokio::select! {
                biased;
                _ = until_stop(shutdown) => return Ok(()),
                _ = until_peer(peer_task) => return Ok(()),
                result = readiness::report_from_env(app, state) => {
                    result.map_err(|error| Error::new("io", error.to_string()))?;
                    reported = true;
                }
            }
        }
        if reported {
            tokio::select! {
                biased;
                _ = until_stop(shutdown) => return Ok(()),
                _ = until_peer(peer_task) => return Ok(()),
            }
        }
        tokio::select! {
            biased;
            _ = until_stop(shutdown) => return Ok(()),
            _ = until_peer(peer_task) => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
    }
}

async fn until_peer(peer_task: &JoinHandle<Result<()>>) {
    while !peer_task.is_finished() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn status_ok(client: &reqwest::Client, url: &str) -> bool {
    let Ok(result) = tokio::time::timeout(Duration::from_millis(500), client.get(url).send()).await
    else {
        return false;
    };
    result.is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
}

fn peer_token(app: &HarnessApp) -> Result<String> {
    let name = app.env("PEER_TOKEN");
    match std::env::var(&name) {
        Ok(token) => Ok(token),
        Err(std::env::VarError::NotPresent) => Err(Error::new("config", format!("missing {name}"))),
        Err(_) => Err(Error::new("config", format!("invalid {name}"))),
    }
}

fn stopped(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn until_stop(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Signal `peer_stop` when `external` becomes true or its sender closes.
async fn forward_shutdown(mut external: watch::Receiver<bool>, peer_stop: watch::Sender<bool>) {
    until_stop(&mut external).await;
    let _ = peer_stop.send(true);
}
