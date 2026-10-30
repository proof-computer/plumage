//! Read-only site for the named tunnel. `GET` and `HEAD` only. No listing.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, Response, StatusCode, header};
use tokio::net::TcpListener;

use crate::error::{Error, Result};

/// Open connections may finish during this window. It stays under the ten
/// seconds the peer has to exit after SIGTERM.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(1);

/// Serve files under `dir` until `shutdown` completes.
///
/// An incomplete request does not hold the server open. After shutdown, open
/// connections get one second to finish, and then the server stops waiting
/// for them.
pub async fn run(
    dir: PathBuf,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let app = Router::new().fallback(handle).with_state(dir);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let shutdown = async move {
        shutdown.await;
        let _ = shutdown_tx.send(());
    };
    let drain = async {
        if shutdown_rx.await.is_ok() {
            tokio::time::sleep(SHUTDOWN_DRAIN).await;
        }
    };
    tokio::select! {
        biased;
        result = axum::serve(listener, app).with_graceful_shutdown(shutdown) => {
            result.map_err(|error| Error::new("io", error.to_string()))?;
        }
        _ = drain => {}
    }
    Ok(())
}

async fn handle(State(dir): State<PathBuf>, request: Request) -> Response<Body> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some((path, kind, cache)) = locate(&dir, request.uri().path()) else {
        return empty(StatusCode::NOT_FOUND);
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return empty(StatusCode::NOT_FOUND),
    };
    file_response(request.method(), bytes, kind, cache)
}

fn locate(root: &Path, raw: &str) -> Option<(PathBuf, &'static str, bool)> {
    if refused(raw) {
        return None;
    }
    let relative = raw.trim_start_matches('/');
    let relative = if raw.ends_with('/') {
        format!("{relative}index.html")
    } else {
        relative.to_owned()
    };
    if relative.is_empty() {
        return None;
    }
    let relative_path = Path::new(&relative);
    if relative_path.is_absolute()
        || !relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let candidate = root.join(relative_path);
    let name = candidate.file_name()?.to_str()?;
    let cache = name == "index.html" || name == "status.json";
    let kind = match candidate.extension().and_then(|ext| ext.to_str())? {
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    };
    let root = root.canonicalize().ok()?;
    let file = candidate.canonicalize().ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    Some((file, kind, cache))
}

/// `..`, a percent-encoded dot, a backslash or a NUL never names a file.
fn refused(raw: &str) -> bool {
    raw.contains('\\')
        || raw.contains('\0')
        || raw.contains("..")
        || raw.to_ascii_lowercase().contains("%2e")
}

fn file_response(
    method: &Method,
    bytes: Vec<u8>,
    content_type: &'static str,
    cache: bool,
) -> Response<Body> {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, bytes.len());
    if cache {
        builder = builder.header(header::CACHE_CONTROL, "no-store");
    }
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    builder.body(body).expect("file response")
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("empty response")
}
