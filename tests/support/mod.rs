//! Stateless fake llama-server shared with the peer and lab tests.
//!
//! `/apply-template` names the context's `you` and the message count.
//! `/tokenize` encodes those two as the first token ids. `/completion` reads
//! them back: an argue on the first step, then a vote for that peer's colour.
//! Nothing is stored between calls.
//!
//! [`spawn`] is the peer test: `p00` votes blue and `p01` votes red, so two
//! peers tie. [`spawn_lab`] is the three-peer lab: `p00` and `p01` vote blue,
//! `p02` votes red, and the argument text names the peer.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use serde_json::{Value, json};

/// Blue for `p00` and any other id. Red for `p01`.
pub fn colour_for(peer_id: &str) -> &'static str {
    if peer_id == "p01" { "red" } else { "blue" }
}

#[derive(Clone, Copy)]
struct Palette {
    lab: bool,
}

// The peer tests call this. The lab test crate compiles this module too and does not.
#[allow(dead_code)]
pub async fn spawn() -> (String, tokio::task::JoinHandle<()>) {
    serve(Palette { lab: false }).await
}

/// Blue for `p00` and `p01`, red for `p02`. The argument text names the peer.
// The lab test calls this. The peer test crate compiles this module too and does not.
#[allow(dead_code)]
pub async fn spawn_lab() -> (String, tokio::task::JoinHandle<()>) {
    serve(Palette { lab: true }).await
}

async fn serve(palette: Palette) -> (String, tokio::task::JoinHandle<()>) {
    let router = Router::new()
        .route("/apply-template", post(apply_template))
        .route("/tokenize", post(tokenize))
        .route("/completion", post(completion))
        .with_state(palette);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}

async fn apply_template(Json(request): Json<Value>) -> Json<Value> {
    let messages = request["messages"].as_array().expect("messages");
    let content = messages[1]["content"].as_str().expect("user content");
    let context: Value = serde_json::from_str(content).expect("context json");
    let you = context["you"].as_str().expect("you");
    Json(json!({"prompt": format!("{you} {}", messages.len())}))
}

async fn tokenize(Json(request): Json<Value>) -> Json<Value> {
    let content = request["content"].as_str().unwrap_or("");
    if let Some((you, count)) = content.split_once(' ')
        && !you.contains('{')
        && let Some(id) = encode_peer(you)
        && let Ok(count) = count.parse::<i32>()
        && count >= 0
    {
        return Json(json!({"tokens": [id, count]}));
    }
    Json(json!({"tokens": [1]}))
}

async fn completion(State(palette): State<Palette>, Json(request): Json<Value>) -> Json<Value> {
    let prompt = request["prompt"].as_array().expect("prompt");
    let id = prompt.first().and_then(Value::as_i64).unwrap_or(0) as i32;
    let messages = prompt.get(1).and_then(Value::as_i64).unwrap_or(0);
    let you = decode_peer(id).unwrap_or_else(|| "p00".to_owned());
    let action = if messages <= 2 {
        let text = if palette.lab {
            format!("argument from {you}")
        } else {
            "today".to_owned()
        };
        json!({"action":"argue","text": text, "cites":[]})
    } else {
        let colour = if palette.lab {
            if you == "p02" { "red" } else { "blue" }
        } else {
            colour_for(&you)
        };
        json!({"action":"vote","colour": colour})
    };
    Json(json!({
        "content": action.to_string(),
        "truncated": false,
        "timings": {"predicted_n": 4},
        "tokens": [1, 2, 3, 4]
    }))
}

fn encode_peer(id: &str) -> Option<i32> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 3 || !bytes.iter().all(u8::is_ascii) {
        return None;
    }
    let mut token = bytes.len() as i32;
    for (index, byte) in bytes.iter().enumerate() {
        token |= i32::from(*byte) << (8 * (index + 1));
    }
    Some(token)
}

fn decode_peer(token: i32) -> Option<String> {
    if token < 0 {
        return None;
    }
    let length = usize::try_from(token & 0xff).ok()?;
    if !(1..=3).contains(&length) {
        return None;
    }
    let mut bytes = Vec::with_capacity(length);
    for index in 0..length {
        bytes.push(((token >> (8 * (index + 1))) & 0xff) as u8);
    }
    String::from_utf8(bytes).ok()
}
