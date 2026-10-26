//! The one place plumage touches the outside web: two allowlisted JSON sources,
//! read into bounded evidence. Fetched text stays data.

use std::{fmt, future::Future, time::Duration};

use serde_json::Value;
use url::Url;

use crate::{
    actions::seed_offset,
    day::Day,
    evidence::{Allowlist, Evidence, SUNRISE_HOST, WIKIMEDIA_HOST},
};

/// Refuse a response body larger than this, counted while it is read.
pub const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// Wikimedia's API asks for an identifying agent.
pub const USER_AGENT: &str = concat!(
    "plumage/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/proof-computer/plumage)"
);

const SUNRISE_FIELDS: [&str; 10] = [
    "sunrise",
    "sunset",
    "solar_noon",
    "day_length",
    "civil_twilight_begin",
    "civil_twilight_end",
    "nautical_twilight_begin",
    "nautical_twilight_end",
    "astronomical_twilight_begin",
    "astronomical_twilight_end",
];

/// The two owner-approved URLs for `day`, both shapes [`Allowlist::permits`].
pub fn sources(day: &Day) -> [Url; 2] {
    let (month, day_of_month) = day.month_day();
    let wikimedia = format!(
        "https://{WIKIMEDIA_HOST}/feed/v1/wikipedia/en/onthisday/all/{month:02}/{day_of_month:02}"
    );
    let sunrise =
        format!("https://{SUNRISE_HOST}/json?lat=51.5074&lng=-0.1278&date={day}&formatted=0");
    [parse_source(&wikimedia), parse_source(&sunrise)]
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

/// HTTPS client for the two allowlisted hosts. Redirects and proxies are off.
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
    if !Allowlist::permits(url) {
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
/// Wikimedia walks the kept event list from `seed_offset(author, len) + prior`.
/// Sunrise ignores `prior`.
pub fn extract(url: &Url, body: &[u8], author: &str, prior: usize) -> Result<String, FetchError> {
    if !Allowlist::permits(url) {
        return Err(failure("source_invalid", format!("refused {url}")));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| failure("source_invalid", "response is not JSON"))?;
    match url.host_str() {
        Some(WIKIMEDIA_HOST) => extract_wikimedia(&value, author, prior),
        Some(SUNRISE_HOST) => extract_sunrise(&value),
        _ => Err(failure("source_invalid", format!("refused {url}"))),
    }
}

fn extract_wikimedia(value: &Value, author: &str, prior: usize) -> Result<String, FetchError> {
    let Some(events) = value.get("events").and_then(Value::as_array) else {
        return Err(failure("source_invalid", "events is not an array"));
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
        return Err(failure("source_invalid", "events contain no text"));
    }
    // Q-20261023-7xbm: the first fetch starts at sha256(author) mod len.
    // `as u128` is lossless: a `usize` is never wider than `u128`.
    let start = (seed_offset(author, len) as u128 + prior as u128) % (len as u128);
    let start = usize::try_from(start).expect("start is below len");
    let mut lines = Vec::with_capacity(len);
    for index in 0..len {
        lines.push(kept[(start + index) % len].as_str());
    }
    Ok(lines.join("\n"))
}

fn extract_sunrise(value: &Value) -> Result<String, FetchError> {
    if value.get("status").and_then(Value::as_str) != Some("OK") {
        return Err(failure("source_invalid", "sunrise status is not OK"));
    }
    let Some(results) = value.get("results").and_then(Value::as_object) else {
        return Err(failure(
            "source_invalid",
            "sunrise results are not an object",
        ));
    };
    let mut lines = Vec::new();
    for key in SUNRISE_FIELDS {
        let Some(field) = results.get(key) else {
            continue;
        };
        let Some(text) = scalar_text(field) else {
            continue;
        };
        lines.push(format!("{key}: {text}"));
    }
    Ok(lines.join("\n"))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Allowlist, then `get`, then [`extract`], then [`Evidence::new`].
///
/// The caller counts an `Ok` against `FETCHES_PER_ACTIVATION` and supplies
/// `prior` (earlier evidence for this URL by `author` in this run).
pub async fn fetch<S: Source>(
    source: &S,
    url: &Url,
    author: &str,
    run: &str,
    prior: usize,
    fetched_at_ms: u64,
) -> Result<Evidence, FetchError> {
    if !Allowlist::permits(url) {
        return Err(failure("not_allowed", format!("refused {url}")));
    }
    let body = source.get(url).await?;
    let text = extract(url, &body, author, prior)?;
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
    fn sources_for_a_real_day_are_permitted() {
        let urls = sources(&day("2026-10-26"));
        assert_eq!(
            urls[0].as_str(),
            "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/10/26"
        );
        assert_eq!(
            urls[1].as_str(),
            "https://api.sunrise-sunset.org/json?lat=51.5074&lng=-0.1278&date=2026-10-26&formatted=0"
        );
        assert!(Allowlist::permits(&urls[0]));
        assert!(Allowlist::permits(&urls[1]));
        assert!("2026-02-29".parse::<Day>().is_err());
        let leap = day("2028-02-29");
        assert_eq!(leap.next().to_string(), "2028-03-01");
        assert!(Allowlist::permits(&sources(&leap)[0]));
        assert!(Allowlist::permits(&sources(&leap)[1]));
    }

    #[test]
    fn wikimedia_events_rotate_from_the_peer_seed() {
        let url = &sources(&day("2026-10-26"))[0];
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
    fn sunrise_lines_follow_the_field_order() {
        let url = &sources(&day("2026-10-26"))[1];
        let body = br#"{
            "status": "OK",
            "results": {
                "day_length": 36000,
                "sunset": "2026-10-26T16:35:00+00:00",
                "sunrise": "2026-10-26T06:35:00+00:00",
                "solar_noon": "2026-10-26T11:35:00+00:00",
                "civil_twilight_end": "2026-10-26T17:10:00+00:00",
                "civil_twilight_begin": "2026-10-26T06:00:00+00:00",
                "nautical_twilight_begin": "2026-10-26T05:20:00+00:00",
                "nautical_twilight_end": "2026-10-26T17:50:00+00:00",
                "astronomical_twilight_begin": "2026-10-26T04:40:00+00:00",
                "astronomical_twilight_end": "2026-10-26T18:30:00+00:00",
                "ignored": true
            }
        }"#;
        let text = extract(url, body, "a", 4).unwrap();
        assert_eq!(text, extract(url, body, "a", 0).unwrap());
        assert_eq!(
            text,
            "\
sunrise: 2026-10-26T06:35:00+00:00
sunset: 2026-10-26T16:35:00+00:00
solar_noon: 2026-10-26T11:35:00+00:00
day_length: 36000
civil_twilight_begin: 2026-10-26T06:00:00+00:00
civil_twilight_end: 2026-10-26T17:10:00+00:00
nautical_twilight_begin: 2026-10-26T05:20:00+00:00
nautical_twilight_end: 2026-10-26T17:50:00+00:00
astronomical_twilight_begin: 2026-10-26T04:40:00+00:00
astronomical_twilight_end: 2026-10-26T18:30:00+00:00"
        );
        let invalid = br#"{"status":"INVALID_DATE","results":{"sunrise":"x"}}"#;
        assert_eq!(
            extract(url, invalid, "a", 0).unwrap_err().code,
            "source_invalid"
        );
    }

    #[tokio::test]
    async fn fetch_refuses_unlisted_urls_without_calling_the_source() {
        let url = Url::parse("https://example.com/").unwrap();
        let source = CountingSource {
            body: Vec::new(),
            calls: AtomicUsize::new(0),
        };
        let error = fetch(&source, &url, "a", "run", 0, 0).await.unwrap_err();
        assert_eq!(error.code, "not_allowed");
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            extract(&url, b"{}", "a", 0).unwrap_err().code,
            "source_invalid"
        );
    }

    #[tokio::test]
    async fn fetch_stores_the_digest_of_the_whole_text() {
        let url = sources(&day("2026-10-26"))[0].clone();
        let prose = "y".repeat(700);
        let body = serde_json::to_vec(&serde_json::json!({
            "events": [{"year": 2025, "text": prose}]
        }))
        .unwrap();
        let source = CountingSource {
            body: body.clone(),
            calls: AtomicUsize::new(0),
        };
        let record = fetch(&source, &url, "a", "run", 0, 1_761_436_800_000)
            .await
            .unwrap();
        let text = extract(&url, &body, "a", 0).unwrap();
        assert!(text.chars().count() > 600);
        assert_eq!(
            record.sha256(),
            format!("{:x}", Sha256::digest(text.as_bytes()))
        );
        assert_eq!(record.excerpt().chars().count(), 600);
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
}
