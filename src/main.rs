//! plumage: a pointless swarm that must agree on a colour each day.
//!
//! Nothing runs yet. The peer and its site arrive as the first
//! consumer of the `forthebirds` library; see `README.md`.

pub mod actions;
pub mod closing;
pub mod colour;
pub mod evidence;

fn main() {
    println!("plumage {}: nothing to do yet", env!("CARGO_PKG_VERSION"));
}
