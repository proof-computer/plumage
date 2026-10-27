//! Read-only site for the named tunnel. `GET` and `HEAD` only. No listing.

use std::path::{Component, Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, Response, StatusCode, header};
use tokio::net::TcpListener;

use crate::error::{Error, Result};

/// Serve files under `dir` until `shutdown` completes.
pub async fn run(
    dir: PathBuf,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let app = Router::new().fallback(handle).with_state(dir);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|error| Error::new("io", error.to_string()))?;
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
    let relative = if raw == "/" {
        "index.html"
    } else {
        raw.trim_start_matches('/')
    };
    if relative.is_empty() {
        return None;
    }
    let relative_path = Path::new(relative);
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
