//! The one place plumage touches the outside web: the allowlisted lists on one
//! host, read into bounded evidence, and one compiled calendar answered
//! without a socket. Fetched text stays data.

use std::{fmt, future::Future, path::PathBuf, time::Duration};

use serde_json::Value;
use url::Url;

use crate::{
    actions::seed_offset,
    day::Day,
    evidence::{Allowlist, EDITIONS, Evidence, LISTS, WIKIMEDIA_HOST},
    observances,
};

/// Refuse a response body larger than this, counted while it is read.
pub const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// Wikimedia's API asks for an identifying agent.
pub const USER_AGENT: &str = concat!(
    "plumage/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/proof-computer/plumage)"
);

/// The Wikipedia edition selected by the peer id's SHA-256 reduction.
pub fn edition_for(peer_id: &str) -> &'static str {
    EDITIONS[seed_offset(peer_id, EDITIONS.len())]
}

/// The owner-approved URLs for `day` and this peer, including the calendar when it names the day.
pub fn sources(day: &Day, peer_id: &str) -> Vec<Url> {
    let (month, day_of_month) = day.month_day();
    let edition = edition_for(peer_id);
    let mut sources: Vec<Url> = LISTS.into_iter().map(|list| parse_source(&format!(
        "https://{WIKIMEDIA_HOST}/feed/v1/wikipedia/{edition}/onthisday/{list}/{month:02}/{day_of_month:02}"
    ))).collect();
    if !observances::for_day(day).is_empty() {
        sources.push(parse_source(observances::URL));
    }
    sources
}

fn parse_source(url: &str) -> Url {
    Url::parse(url).expect("allowlisted source URL is valid")
}

/// A fetch that did not become evidence.
///
/// `code` is one of `not_allowed`, `fetch_failed`, `fetch_status`, `too_large`,
/// or `source_invalid`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FetchError {}

fn failure(code: &'static str, message: impl Into<String>) -> FetchError {
    FetchError {
        code,
        message: message.into(),
    }
}

/// Offline seam. `HttpSource` is the live client; tests and the lab supply their own.
pub trait Source: Send + Sync {
    fn get(&self, url: &Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send;
}

/// Offline responses, selected by the allowlisted URL's host.
pub struct FixtureSource {
    dir: PathBuf,
}

impl FixtureSource {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Source for FixtureSource {
    async fn get(&self, url: &Url) -> Result<Vec<u8>, FetchError> {
        if !Allowlist::permits(url) {
            return Err(failure("not_allowed", format!("refused {url}")));
        }
        let host = url.host_str().expect("allowlisted URL has a host");
        tokio::fs::read(self.dir.join(format!("{host}.json")))
            .await
            .map_err(|error| failure("fetch_failed", error.to_string()))
    }
}

/// HTTPS client for five allowlisted lists on one host. Redirects and proxies are off.
#[derive(Clone, Debug)]
pub struct HttpSource {
    client: reqwest::Client,
}

impl HttpSource {
    /// `timeout` bounds the whole request. The connect timeout is 10 seconds.
    pub fn new(timeout: Duration) -> Result<Self, FetchError> {
        reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(timeout)
            .build()
            .map(|client| Self { client })
            .map_err(|error| failure("fetch_failed", error.to_string()))
    }
}

impl Source for HttpSource {
    fn get(&self, url: &Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
        let client = self.client.clone();
        let url = url.clone();
        async move { read_limited(&client, &url).await }
    }
}

/// Allowlist first, so a refused URL never opens a socket.
async fn read_limited(client: &reqwest::Client, url: &Url) -> Result<Vec<u8>, FetchError> {
    if !Allowlist::permits_live(url) {
        return Err(failure("not_allowed", format!("refused {url}")));
    }
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| failure("fetch_failed", error.to_string()))?;
    if !response.status().is_success() {
        return Err(failure(
            "fetch_status",
            format!("status {}", response.status()),
        ));
    }
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| failure("fetch_failed", error.to_string()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err(failure(
                "too_large",
                format!("body exceeds {BODY_LIMIT} bytes"),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Text fields of one response, and nothing else.
///
/// `prior` is how many times `author` already fetched this URL in the run.
/// Each list walks its kept entries from `seed_offset(author, len) + prior`.
pub fn extract(url: &Url, body: &[u8], author: &str, prior: usize) -> Result<String, FetchError> {
    if !Allowlist::permits(url) {
        return Err(failure("source_invalid", format!("refused {url}")));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| failure("source_invalid", "response is not JSON"))?;
    match Allowlist::list(url) {
        Some(list) => extract_list(&value, list, author, prior),
        _ => Err(failure("source_invalid", format!("refused {url}"))),
    }
}

fn extract_list(
    value: &Value,
    list: &str,
    author: &str,
    prior: usize,
) -> Result<String, FetchError> {
    let Some(events) = value.get(list).and_then(Value::as_array) else {
        return Err(failure("source_invalid", format!("{list} is not an array")));
    };
    let mut kept = Vec::new();
    for event in events {
        let Some(text) = event.get("text").and_then(Value::as_str) else {
            continue;
        };
        // A non-integer year is omitted; the text is still a kept event.
        let line = match event.get("year").and_then(Value::as_i64) {
            Some(year) => format!("{year}: {text}"),
            None => text.to_owned(),
        };
        kept.push(line);
    }
    let len = kept.len();
    if len == 0 {
        return Err(failure(
            "source_invalid",
            format!("{list} contains no text"),
        ));
    }
    Ok(rotated(&kept, author, prior))
}

fn rotated(kept: &[String], author: &str, prior: usize) -> String {
    let len = kept.len();
    // Q-20261023-7xbm: the first fetch starts at sha256(author) mod len.
    // `as u128` is lossless: a `usize` is never wider than `u128`.
    let start = (seed_offset(author, len) as u128 + prior as u128) % (len as u128);
    let start = usize::try_from(start).expect("start is below len");
    let mut lines = Vec::with_capacity(len);
    for index in 0..len {
        lines.push(kept[(start + index) % len].as_str());
    }
    lines.join("\n")
}

/// Allowlist, then the compiled calendar or `get` and [`extract`], then [`Evidence::new`].
///
/// The caller counts an `Ok` against `FETCHES_PER_ACTIVATION` and supplies
/// `prior` (earlier evidence for this URL by `author` in this run).
pub async fn fetch<S: Source>(
    source: &S,
    url: &Url,
    day: &Day,
    author: &str,
    run: &str,
    prior: usize,
    fetched_at_ms: u64,
) -> Result<Evidence, FetchError> {
    if !Allowlist::permits(url) {
        return Err(failure("not_allowed", format!("refused {url}")));
    }
    let text = if Allowlist::is_observances(url) {
        let kept: Vec<_> = observances::for_day(day)
            .into_iter()
            .map(observances::line)
            .collect();
        if kept.is_empty() {
            return Err(failure(
                "source_invalid",
                format!("no UN observance on {day}"),
            ));
        }
        rotated(&kept, author, prior)
    } else {
        let body = source.get(url).await?;
        extract(url, &body, author, prior)?
    };
    Ok(Evidence::new(
        author.to_owned(),
        run.to_owned(),
        url.clone(),
        fetched_at_ms,
        &text,
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        io::ErrorKind,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use sha2::{Digest, Sha256};

    use super::*;

    struct CountingSource {
        body: Vec<u8>,
        calls: AtomicUsize,
    }

    impl Source for CountingSource {
        fn get(&self, _url: &Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body = self.body.clone();
            async move { Ok(body) }
        }
    }

    fn day(text: &str) -> Day {
        text.parse().unwrap()
    }

    #[test]
    fn editions_are_selected_by_the_whole_peer_id_hash() {
        for (peer, expected) in [("p00", "en"), ("b", "de"), ("p01", "fr"), ("a", "es")] {
            assert_eq!(edition_for(peer), expected);
        }
        for peer in ["p00", "p01", "p02", "p03", "a", "b", "c", "d"] {
            assert_eq!(edition_for(peer), EDITIONS[seed_offset(peer, 4)]);
        }
    }

    #[test]
    fn sources_for_a_real_day_are_the_peers_five_permitted_lists_in_order() {
        for (date, suffix) in [("2026-10-26", "10/26"), ("2028-02-29", "02/29")] {
            for peer in ["p00", "b", "p01", "a"] {
                let urls = sources(&day(date), peer);
                assert_eq!(urls.len(), 5);
                for (url, list) in urls.iter().zip(LISTS) {
                    let edition = edition_for(peer);
                    assert_eq!(
                        url.as_str(),
                        format!(
                            "https://api.wikimedia.org/feed/v1/wikipedia/{edition}/onthisday/{list}/{suffix}"
                        )
                    );
                    assert!(Allowlist::permits(url));
                    assert_eq!(Allowlist::source(url), Some((edition, list)));
                }
            }
        }
        assert!("2026-02-29".parse::<Day>().is_err());
    }

    #[test]
    fn german_births_rotate_as_english_and_reject_the_wrong_list() {
        let body = br#"{"births":[{"year":1903,"text":"Erster"},{"text":"Zweiter"},{"year":1955,"text":"Dritter"}]}"#;
        let kept = ["1903: Erster", "Zweiter", "1955: Dritter"];
        let german = &sources(&day("2026-10-26"), "b")[1];
        let english = &sources(&day("2026-10-26"), "p00")[1];
        for prior in [0, 1] {
            let text = extract(german, body, "a", prior).unwrap();
            assert_eq!(text.lines().count(), 3);
            assert_eq!(
                text.lines().next().unwrap(),
                kept[(seed_offset("a", 3) + prior) % 3]
            );
            assert_eq!(text, extract(english, body, "a", prior).unwrap());
        }
        assert_eq!(
            extract(german, br#"{"events":[{"text":"fact"}]}"#, "a", 0)
                .unwrap_err()
                .code,
            "source_invalid"
        );
    }

    #[test]
    fn wikimedia_events_rotate_from_the_peer_seed() {
        let url = &sources(&day("2026-10-26"), "p00")[0];
        let body = serde_json::to_vec(&serde_json::json!({
            "events": [
                {"year": 1901, "text": "one"},
                {"text": "two"},
                {"year": 1903, "text": 5},
                {"year": "1904", "text": "four"},
                {"year": 1905, "text": "five"},
                {"year": 1906, "text": "six"}
            ]
        }))
        .unwrap();
        let kept = ["1901: one", "two", "four", "1905: five", "1906: six"];
        let start = seed_offset("a", kept.len());
        let first = extract(url, &body, "a", 0).unwrap();
        let second = extract(url, &body, "a", 1).unwrap();
        let first_lines: Vec<_> = first.lines().collect();
        let second_lines: Vec<_> = second.lines().collect();
        assert_eq!(first_lines.len(), 5);
        assert_eq!(first_lines[0], kept[start]);
        assert_eq!(second_lines[0], first_lines[1]);
        assert_eq!(*second_lines.last().unwrap(), first_lines[0]);
        for index in 0..kept.len() {
            assert_eq!(first_lines[index], kept[(start + index) % kept.len()]);
            assert_eq!(second_lines[index], kept[(start + 1 + index) % kept.len()]);
        }
        assert_eq!(
            extract(url, br#"{"events":[]}"#, "a", 0).unwrap_err().code,
            "source_invalid"
        );
        assert_eq!(
            extract(url, br#"{"events":[{"text":1}]}"#, "a", 0)
                .unwrap_err()
                .code,
            "source_invalid"
        );
        assert_eq!(
            extract(url, b"not-json", "a", 0).unwrap_err().code,
            "source_invalid"
        );
    }

    #[test]
    fn every_list_extracts_text_rotates_independently_and_rejects_invalid_shapes() {
        for (url, list) in sources(&day("2026-10-26"), "p00").iter().zip(LISTS) {
            let entries = serde_json::json!([
                {"year":1901,"text":"one"}, {"text":"two"},
                {"year":1903,"text":5}, {"year":"1904","text":"four"},
                {"year":1905,"text":"five"}, {"year":1906,"text":"six"},
                {"year":1.5,"text":"fractional"}
            ]);
            let body = serde_json::to_vec(&serde_json::json!({list:entries})).unwrap();
            let kept = [
                "1901: one",
                "two",
                "four",
                "1905: five",
                "1906: six",
                "fractional",
            ];
            for prior in [0, 1, usize::MAX] {
                let text = extract(url, &body, "a", prior).unwrap();
                let start = ((seed_offset("a", kept.len()) as u128 + prior as u128)
                    % kept.len() as u128) as usize;
                assert_eq!(
                    text,
                    (0..kept.len())
                        .map(|i| kept[(start + i) % kept.len()])
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            let other = if list == "events" { "births" } else { "events" };
            for invalid in [
                serde_json::json!({other:entries}),
                serde_json::json!({list:[]}),
                serde_json::json!({list:[{"text":1}]}),
                serde_json::json!({list:{}}),
            ] {
                assert_eq!(
                    extract(url, &serde_json::to_vec(&invalid).unwrap(), "a", 0)
                        .unwrap_err()
                        .code,
                    "source_invalid"
                );
            }
        }
        let url = &sources(&day("2026-10-26"), "p00")[1];
        let body = serde_json::to_vec(&serde_json::json!({"births": (0..6).map(|i| serde_json::json!({"text":i.to_string()})).collect::<Vec<_>>()})).unwrap();
        assert_eq!(
            extract(url, &body, "a", 0).unwrap().lines().next().unwrap(),
            seed_offset("a", 6).to_string()
        );
        let url = &sources(&day("2026-10-26"), "p00")[3];
        assert_eq!(
            extract(url, br#"{"holidays":[{"text":"A holiday"}]}"#, "a", 0).unwrap(),
            "A holiday"
        );
    }

    #[tokio::test]
    async fn fixture_source_reads_only_allowlisted_host_files() {
        let dir = tempfile::tempdir().unwrap();
        let body = include_bytes!("../tests/fixtures/api.wikimedia.org.json");
        let file = dir.path().join("api.wikimedia.org.json");
        std::fs::write(&file, body).unwrap();
        let source = FixtureSource::new(dir.path());
        let urls = sources(&day("2026-10-26"), "p00");
        assert_eq!(source.get(&urls[0]).await.unwrap(), body);
        for peer in ["p00", "b"] {
            let urls = sources(&day("2026-10-26"), peer);
            for url in &urls {
                assert_eq!(source.get(url).await.unwrap(), body);
            }
            let evidence = fetch(&source, &urls[0], &day("2026-10-26"), peer, "run", 0, 0)
                .await
                .unwrap();
            assert_eq!(evidence.url(), &urls[0]);
            for url in &urls[1..] {
                assert_eq!(
                    fetch(&source, url, &day("2026-10-26"), peer, "run", 0, 0)
                        .await
                        .unwrap_err()
                        .code,
                    "source_invalid"
                );
            }
        }
        std::fs::remove_file(&file).unwrap();
        assert_eq!(source.get(&urls[0]).await.unwrap_err().code, "fetch_failed");
        std::fs::write(&file, body).unwrap();
        // A directory path that is a file cannot be read as a fixture directory.
        // A refused URL must still fail at the allowlist before any file read.
        let unreadable = FixtureSource::new(file);
        assert_eq!(
            unreadable.get(&urls[0]).await.unwrap_err().code,
            "fetch_failed"
        );
        let refused = Url::parse("https://api.wikimedia.org/not-allowlisted").unwrap();
        assert_eq!(
            unreadable.get(&refused).await.unwrap_err().code,
            "not_allowed"
        );
    }

    #[tokio::test]
    async fn fetch_refuses_unlisted_urls_without_calling_the_source() {
        let source = CountingSource {
            body: Vec::new(),
            calls: AtomicUsize::new(0),
        };
        for text in [
            "https://example.com/",
            "https://api.wikimedia.org/feed/v1/wikipedia/it/onthisday/events/10/26",
            "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/10/26",
            "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/events/10/26?x=1",
        ] {
            let url = Url::parse(text).unwrap();
            assert_eq!(
                fetch(&source, &url, &day("2026-10-26"), "a", "run", 0, 0)
                    .await
                    .unwrap_err()
                    .code,
                "not_allowed"
            );
            assert_eq!(
                extract(&url, b"{}", "a", 0).unwrap_err().code,
                "source_invalid"
            );
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn fetch_stores_the_digest_of_the_whole_text() {
        let url = sources(&day("2026-10-26"), "p00")[0].clone();
        let prose = "y".repeat(700);
        let body = serde_json::to_vec(&serde_json::json!({
            "events": [{"year": 2025, "text": prose}]
        }))
        .unwrap();
        let source = CountingSource {
            body: body.clone(),
            calls: AtomicUsize::new(0),
        };
        let record = fetch(
            &source,
            &url,
            &day("2026-10-26"),
            "a",
            "run",
            0,
            1_761_436_800_000,
        )
        .await
        .unwrap();
        let text = extract(&url, &body, "a", 0).unwrap();
        assert!(text.chars().count() > 600);
        assert_eq!(
            record.sha256(),
            format!("{:x}", Sha256::digest(text.as_bytes()))
        );
        assert_eq!(record.excerpt().chars().count(), 600);
        assert!(record.excerpt().ends_with('…'));
        assert_ne!(
            record.sha256(),
            format!("{:x}", Sha256::digest(record.excerpt().as_bytes()))
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn http_source_does_not_connect_to_a_refused_url() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}/json")).unwrap();
        let source = HttpSource::new(Duration::from_secs(2)).unwrap();
        let error = source.get(&url).await.unwrap_err();
        assert_eq!(error.code, "not_allowed");
        match listener.accept() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            other => panic!("listener accepted a connection: {other:?}"),
        }
    }

    #[test]
    fn calendar_is_offered_only_on_a_named_day_after_the_five_lists() {
        for peer in ["p00", "b", "p01", "a"] {
            let urls = sources(&day("2026-02-02"), peer);
            assert_eq!(urls.len(), 6);
            for (url, list) in urls.iter().zip(LISTS) {
                assert_eq!(Allowlist::source(url), Some((edition_for(peer), list)));
                assert!(Allowlist::permits_live(url));
            }
            assert_eq!(urls[5].as_str(), observances::URL);
            assert!(urls.iter().all(Allowlist::permits));
            assert!(!Allowlist::permits_live(&urls[5]));
            let empty = sources(&day("2026-10-26"), peer);
            assert_eq!(empty.len(), 5);
            assert!(!empty.iter().any(Allowlist::is_observances));
        }
    }

    #[tokio::test]
    async fn calendar_fetches_rotate_and_digest_without_calling_the_source() {
        let source = CountingSource {
            body: Vec::new(),
            calls: AtomicUsize::new(0),
        };
        let day = day("2026-03-21");
        let url = Url::parse(observances::URL).unwrap();
        let kept: Vec<_> = observances::for_day(&day)
            .into_iter()
            .map(observances::line)
            .collect();
        assert_eq!(kept.len(), 7);
        for (author, prior) in [("a", 0), ("a", 1), ("p01", 0), ("a", usize::MAX)] {
            let start = ((seed_offset(author, kept.len()) as u128 + prior as u128)
                % kept.len() as u128) as usize;
            let expected = kept[start..]
                .iter()
                .chain(&kept[..start])
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let record = fetch(&source, &url, &day, author, "run", prior, 0)
                .await
                .unwrap();
            assert_eq!(record.excerpt(), expected);
            assert_eq!(
                record.sha256(),
                format!("{:x}", Sha256::digest(expected.as_bytes()))
            );
            assert_eq!(record.url(), &url);
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unnamed_calendar_day_and_direct_http_are_refused_without_a_source_call() {
        let source = CountingSource {
            body: Vec::new(),
            calls: AtomicUsize::new(0),
        };
        let url = Url::parse(observances::URL).unwrap();
        let error = fetch(&source, &url, &day("2026-10-26"), "a", "run", 0, 0)
            .await
            .unwrap_err();
        assert_eq!(error.code, "source_invalid");
        assert!(error.message.contains("2026-10-26"));
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            extract(&url, b"{}", "a", 0).unwrap_err().code,
            "source_invalid"
        );
        let http = HttpSource::new(Duration::from_secs(1)).unwrap();
        assert_eq!(http.get(&url).await.unwrap_err().code, "not_allowed");
    }
}
