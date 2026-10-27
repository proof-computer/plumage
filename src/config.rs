//! One JSON file names a peer, its day clock, and the peers it syncs with.

use std::net::SocketAddr;
use std::path::PathBuf;

use forthebirds_net::StaticPeer;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::clock::DayClock;
use crate::day::Day;
use crate::error::{Error, Result};

/// `1_000..=3_600_000` on the day clock.
pub const MIN_ACTIVATION_INTERVAL_MS: u64 = 1_000;
pub const MAX_ACTIVATION_INTERVAL_MS: u64 = 3_600_000;
/// `50..=3_600_000` on the wall clock.
pub const MIN_SYNC_INTERVAL_MS: u64 = 50;
pub const MAX_SYNC_INTERVAL_MS: u64 = 3_600_000;
pub const DEFAULT_SYNC_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_MAX_STEPS: u32 = 16;
pub const DEFAULT_CONTEXT_TOKENS: usize = 4_096;
pub const DEFAULT_OUTPUT_TOKENS: u32 = 512;
pub const DEFAULT_INFERENCE_TIMEOUT_SECONDS: u64 = 180;
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_FETCH_TIMEOUT_SECONDS: u64 = 10;
/// Planetarium's upper bound, so writing never binds before reading.
pub const DEFAULT_MEMORY_BUDGET: u64 = 1_000_000;
pub const DEFAULT_MEMORY_ITEM_TOKENS: u64 = 512;
pub const DATABASE_CACHE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DOCUMENTS: usize = 10_000_000;

/// The file `plumage peer` reads.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    pub peer_id: String,
    pub data_dir: PathBuf,
    pub site_dir: PathBuf,
    pub model_url: String,
    #[serde(deserialize_with = "de_socket", serialize_with = "ser_socket")]
    pub listen: SocketAddr,
    pub advertisement_file: PathBuf,
    pub peers: Vec<StaticPeer>,
    pub token: String,
    pub clock: DayClock,
    pub last_day: Option<Day>,
    #[serde(default = "default_activation_interval_ms")]
    pub activation_interval_ms: u64,
    #[serde(default = "default_sync_interval_ms")]
    pub sync_interval_ms: u64,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_context_tokens")]
    pub context_tokens: usize,
    #[serde(default = "default_output_tokens")]
    pub output_tokens: u32,
    #[serde(default = "default_inference_timeout_seconds")]
    pub inference_timeout_seconds: u64,
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_fetch_timeout_seconds")]
    pub fetch_timeout_seconds: u64,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default = "default_memory_budget")]
    pub memory_budget: u64,
    #[serde(default = "default_memory_item_tokens")]
    pub memory_item_tokens: u64,
}

fn default_activation_interval_ms() -> u64 {
    forthebirds_runtime::activation::ACTIVATION_INTERVAL_MS
}

fn default_sync_interval_ms() -> u64 {
    DEFAULT_SYNC_INTERVAL_MS
}

fn default_max_steps() -> u32 {
    DEFAULT_MAX_STEPS
}

fn default_context_tokens() -> usize {
    DEFAULT_CONTEXT_TOKENS
}

fn default_output_tokens() -> u32 {
    DEFAULT_OUTPUT_TOKENS
}

fn default_inference_timeout_seconds() -> u64 {
    DEFAULT_INFERENCE_TIMEOUT_SECONDS
}

fn default_request_timeout_seconds() -> u64 {
    DEFAULT_REQUEST_TIMEOUT_SECONDS
}

fn default_fetch_timeout_seconds() -> u64 {
    DEFAULT_FETCH_TIMEOUT_SECONDS
}

fn default_memory_budget() -> u64 {
    DEFAULT_MEMORY_BUDGET
}

fn default_memory_item_tokens() -> u64 {
    DEFAULT_MEMORY_ITEM_TOKENS
}

fn de_socket<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<SocketAddr, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(serde::de::Error::custom)
}

fn ser_socket<S: Serializer>(
    address: &SocketAddr,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&address.to_string())
}

impl PeerConfig {
    /// Code `config` when an identity, address, token, clock or bound is refused.
    pub fn validate(&self) -> Result<()> {
        forthebirds_store::identifier(&self.peer_id).map_err(|_| {
            Error::new(
                "config",
                "peer_id must be 1–128 ASCII letters, digits, -, _, . or :",
            )
        })?;
        if !self.listen.is_ipv6() {
            return Err(Error::new(
                "config",
                "listen must be an IPv6 socket address",
            ));
        }
        if !(32..=512).contains(&self.token.len())
            || !self.token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Error::new(
                "config",
                "token must be 32–512 visible ASCII bytes",
            ));
        }
        self.clock.validate()?;
        if !(MIN_ACTIVATION_INTERVAL_MS..=MAX_ACTIVATION_INTERVAL_MS)
            .contains(&self.activation_interval_ms)
        {
            return Err(Error::new(
                "config",
                "activation_interval_ms must be from 1000 to 3600000",
            ));
        }
        if !(MIN_SYNC_INTERVAL_MS..=MAX_SYNC_INTERVAL_MS).contains(&self.sync_interval_ms) {
            return Err(Error::new(
                "config",
                "sync_interval_ms must be from 50 to 3600000",
            ));
        }
        if !(1..=32).contains(&self.max_steps) {
            return Err(Error::new("config", "max_steps must be from 1 to 32"));
        }
        if self.context_tokens == 0 || u32::try_from(self.context_tokens).is_err() {
            return Err(Error::new(
                "config",
                "context_tokens must fit below the output bound",
            ));
        }
        if self.output_tokens as usize >= self.context_tokens {
            return Err(Error::new(
                "config",
                "output_tokens must be below context_tokens",
            ));
        }
        if !(1..=1_000_000).contains(&self.memory_budget) {
            return Err(Error::new(
                "config",
                "memory_budget must be from 1 to 1000000",
            ));
        }
        if !(1..=4_096).contains(&self.memory_item_tokens) {
            return Err(Error::new(
                "config",
                "memory_item_tokens must be from 1 to 4096",
            ));
        }
        Ok(())
    }
}
