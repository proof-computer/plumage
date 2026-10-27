//! Bounded fetched data and the two owner-approved JSON source shapes.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

// Q-20261023-xuav: two JSON APIs, no HTML, at most 600 characters per excerpt.
pub const EXCERPT_CHARS: usize = 600;
pub const WIKIMEDIA_HOST: &str = "api.wikimedia.org";
pub const SUNRISE_HOST: &str = "api.sunrise-sunset.org";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EvidenceId(pub String);

/// Immutable evidence: construct from fetched text, retaining its full digest
/// while bounding the excerpt by Unicode scalar values, not UTF-8 bytes.
/// Text remains data; this module neither fetches it nor builds prompts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "EvidenceBody")]
pub struct Evidence {
    id: EvidenceId,
    author: String,
    run: String,
    url: Url,
    fetched_at_ms: u64,
    sha256: String,
    excerpt: String,
}

impl Evidence {
    /// `text` is fetched data extracted from the JSON response's text fields.
    pub fn new(author: String, run: String, url: Url, fetched_at_ms: u64, text: &str) -> Self {
        let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
        let id = derive_id(&run, &author, url.as_str(), &sha256);
        Self {
            id,
            author,
            run,
            url,
            fetched_at_ms,
            sha256,
            excerpt: text.chars().take(EXCERPT_CHARS).collect(),
        }
    }

    pub fn id(&self) -> &EvidenceId {
        &self.id
    }

    pub fn author(&self) -> &str {
        &self.author
    }

    pub fn run(&self) -> &str {
        &self.run
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn fetched_at_ms(&self) -> u64 {
        self.fetched_at_ms
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn excerpt(&self) -> &str {
        &self.excerpt
    }
}

/// Length-prefix the tuple so field boundaries cannot collide.
fn derive_id(run: &str, author: &str, url: &str, sha256: &str) -> EvidenceId {
    let mut identity = Sha256::new();
    for field in [run, author, url, sha256] {
        identity.update((field.len() as u64).to_be_bytes());
        identity.update(field.as_bytes());
    }
    EvidenceId(format!("{:x}", identity.finalize()))
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceBody {
    id: EvidenceId,
    author: String,
    run: String,
    url: Url,
    fetched_at_ms: u64,
    sha256: String,
    excerpt: String,
}

impl TryFrom<EvidenceBody> for Evidence {
    type Error = String;

    fn try_from(body: EvidenceBody) -> Result<Self, Self::Error> {
        if !is_lower_hex(&body.sha256, 64) {
            return Err("sha256 must be 64 lowercase hex digits".into());
        }
        if body.excerpt.chars().count() > EXCERPT_CHARS {
            return Err("excerpt exceeds 600 characters".into());
        }
        if !Allowlist::permits(&body.url) {
            return Err("url is outside the allowlist".into());
        }
        let id = derive_id(&body.run, &body.author, body.url.as_str(), &body.sha256);
        if body.id != id {
            return Err("id does not match the evidence derivation".into());
        }
        Ok(Self {
            id,
            author: body.author,
            run: body.run,
            url: body.url,
            fetched_at_ms: body.fetched_at_ms,
            sha256: body.sha256,
            excerpt: body.excerpt,
        })
    }
}

pub struct Allowlist;

impl Allowlist {
    pub fn permits(url: &Url) -> bool {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.port().is_some()
        {
            return false;
        }
        match url.host_str() {
            Some(WIKIMEDIA_HOST) => {
                url.query().is_none()
                    && url
                        .path()
                        .strip_prefix("/feed/v1/wikipedia/en/onthisday/all/")
                        .is_some_and(|date| {
                            date.len() == 5
                                && date.as_bytes()[2] == b'/'
                                && valid_date(&format!("2000-{}-{}", &date[..2], &date[3..]))
                        })
            }
            Some(SUNRISE_HOST) => {
                url.path() == "/json"
                    && url.query().is_some_and(|query| {
                        query
                            .strip_prefix("lat=51.5074&lng=-0.1278&date=")
                            .and_then(|rest| rest.strip_suffix("&formatted=0"))
                            .is_some_and(valid_date)
                    })
            }
            _ => false,
        }
    }
}

fn valid_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = date[..4].parse().expect("checked digits");
    let month: usize = date[5..7].parse().expect("checked digits");
    let day: u32 = date[8..].parse().expect("checked digits");
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&month) && day >= 1 && day <= days[month - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKI: &str = "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/10/23";
    const SUN: &str =
        "https://api.sunrise-sunset.org/json?lat=51.5074&lng=-0.1278&date=2026-10-23&formatted=0";

    #[test]
    fn permits_only_the_two_exact_source_shapes() {
        for url in [
            WIKI,
            SUN,
            "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/all/02/29",
            "https://api.sunrise-sunset.org/json?lat=51.5074&lng=-0.1278&date=2024-02-29&formatted=0",
        ] {
            assert!(Allowlist::permits(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            WIKI.replace("api.wikimedia.org", "example.com"),
            WIKI.replace("api.wikimedia.org", "api.wikimedia.org.example.com"),
            WIKI.replace("https:", "http:"),
            WIKI.replace("https:", "ftp:"),
            WIKI.replace("https://", "https://reader@"),
            WIKI.replace("https://", "https://:password@"),
            WIKI.replace(".org/", ".org:444/"),
            WIKI.replace("/all/", "/births/"),
            WIKI.replace("/en/", "/de/"),
            WIKI.replace("10/23", "1/23"),
            WIKI.replace("10/23", "10-23"),
            WIKI.replace("10/23", "13/23"),
            WIKI.replace("10/23", "00/23"),
            WIKI.replace("10/23", "10/00"),
            WIKI.replace("10/23", "02/30"),
            WIKI.replace("10/23", "%31%30/23"),
            format!("{WIKI}/"),
            format!("{WIKI}?"),
            format!("{WIKI}?x=1"),
            format!("{WIKI}#"),
            format!("{SUN}#fragment"),
            SUN.replace("/json?", "/json/?"),
            SUN.replace("lat=51.5074", "lat=0"),
            SUN.replace("&lng=-0.1278", ""),
            SUN.replace("lat=51.5074&lng=-0.1278", "lng=-0.1278&lat=51.5074"),
            SUN.replace("2026-10-23", "2026-1-23"),
            SUN.replace("2026-10-23", "2026-02-29"),
            SUN.replace("formatted=0", "formatted=1"),
            format!("{SUN}&extra=1"),
            format!("{SUN}&formatted=0"),
            "https://api.sunrise-sunset.org/json".to_owned(),
        ] {
            assert!(!Allowlist::permits(&Url::parse(&url).unwrap()), "{url}");
        }
    }

    fn evidence(author: &str, run: &str, url: &str, at: u64, text: &str) -> Evidence {
        Evidence::new(
            author.into(),
            run.into(),
            Url::parse(url).unwrap(),
            at,
            text,
        )
    }

    #[test]
    fn evidence_bounds_unicode_and_digests_the_untruncated_text() {
        let text = format!("{}tail", "🦜".repeat(EXCERPT_CHARS));
        let record = evidence("a", "run", WIKI, 0, &text);
        assert_eq!(record.excerpt().chars().count(), EXCERPT_CHARS);
        assert_eq!(
            record.sha256(),
            format!("{:x}", Sha256::digest(text.as_bytes()))
        );
        assert_ne!(
            record.sha256(),
            format!("{:x}", Sha256::digest(record.excerpt().as_bytes()))
        );
        assert_eq!(record.author(), "a");
        assert_eq!(record.run(), "run");
        assert_eq!(record.url().as_str(), WIKI);
        assert_eq!(record.fetched_at_ms(), 0);
        assert_eq!(
            serde_json::to_value(&record).unwrap()["excerpt"],
            record.excerpt()
        );
        assert_eq!(evidence("a", "run", WIKI, 1, "").excerpt(), "");
        assert_eq!(
            evidence("a", "run", WIKI, 1, "abc").sha256(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identity_uses_exactly_run_author_url_and_full_digest() {
        let first = evidence("a", "bc", WIKI, 1, "same");
        assert_eq!(first.id(), evidence("a", "bc", WIKI, 2, "same").id());
        for other in [
            evidence("a", "other", WIKI, 1, "same"),
            evidence("other", "bc", WIKI, 1, "same"),
            evidence("a", "bc", SUN, 1, "same"),
            evidence("a", "bc", WIKI, 1, "different"),
            evidence("ca", "b", WIKI, 1, "same"),
        ] {
            assert_ne!(first.id(), other.id());
        }
        let prefix = "x".repeat(EXCERPT_CHARS);
        assert_ne!(
            evidence("a", "r", WIKI, 0, &prefix).id(),
            evidence("a", "r", WIKI, 0, &format!("{prefix}tail")).id()
        );
    }

    #[test]
    fn evidence_round_trips_and_refuses_a_changed_id_excerpt_or_url() {
        let record = evidence("a", "run", WIKI, 7, "same");
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(
            serde_json::from_value::<Evidence>(value.clone()).unwrap(),
            record
        );

        let mut changed_id = value.clone();
        changed_id["id"] = serde_json::json!("0".repeat(64));
        assert!(serde_json::from_value::<Evidence>(changed_id).is_err());

        let mut long_excerpt = value.clone();
        long_excerpt["excerpt"] = serde_json::json!("x".repeat(EXCERPT_CHARS + 1));
        assert!(serde_json::from_value::<Evidence>(long_excerpt).is_err());

        let mut malformed_sha = value.clone();
        malformed_sha["sha256"] = serde_json::json!("zz".repeat(32));
        assert!(serde_json::from_value::<Evidence>(malformed_sha).is_err());

        let mut uppercase_sha = value.clone();
        uppercase_sha["sha256"] = serde_json::json!(record.sha256().to_ascii_uppercase());
        assert!(serde_json::from_value::<Evidence>(uppercase_sha).is_err());

        let mut unknown_field = value.clone();
        unknown_field["note"] = serde_json::json!("extra");
        assert!(serde_json::from_value::<Evidence>(unknown_field).is_err());

        let mut foreign_url = value;
        foreign_url["url"] = serde_json::json!("https://example.com/");
        assert!(serde_json::from_value::<Evidence>(foreign_url).is_err());
    }
}
