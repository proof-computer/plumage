//! `plumage launch`: ask the Cargo harness to fetch the payload and start the peer.
//!
//! The launcher does not write a peer config, supervise `plumage peer`, or restart it.

use std::future::Future;

use liskov_cargo_harness::launch::{Source, launch};

use crate::error::Result;
use crate::liskov;

/// Run until `shutdown` completes. A harness error keeps its code.
pub async fn run(shutdown: impl Future<Output = ()> + Send) -> Result<()> {
    let app = liskov::app()?;
    let source = Source::from_env(&app)?;
    launch(&app, source, shutdown).await?;
    Ok(())
}
