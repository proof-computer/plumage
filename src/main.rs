//! plumage: a pointless swarm that must agree on a colour each day.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use plumage::config::PeerConfig;
use plumage::error::Error;
use plumage::fetch::HttpSource;
use plumage::lab;
use plumage::peer;
use tokio::sync::watch;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some("peer") => {
            let Some(path) = args.next() else {
                eprintln!("config: missing config.json");
                return ExitCode::from(1);
            };
            if args.next().is_some() {
                eprintln!("config: unexpected argument");
                return ExitCode::from(1);
            }
            match run_peer(&path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        Some("lab") => {
            let rest: Vec<String> = args.collect();
            match run_lab(&rest) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        Some(_) => {
            print_usage();
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    println!("plumage {}", env!("CARGO_PKG_VERSION"));
    println!("plumage peer <config.json>");
    println!(
        "plumage lab --dir <path> [--peers <n>] [--model-url <url>] [--day <YYYY-MM-DD>] [--days <n>] [--speed <n>]"
    );
}

#[tokio::main]
async fn run_peer(path: &str) -> Result<(), Error> {
    let text =
        std::fs::read_to_string(path).map_err(|error| Error::new("config", error.to_string()))?;
    let config: PeerConfig =
        serde_json::from_str(&text).map_err(|error| Error::new("config", error.to_string()))?;
    config.validate()?;
    let source = HttpSource::new(Duration::from_secs(config.fetch_timeout_seconds))?;
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(async move {
        watch_signals(sender).await;
    });
    peer::run(config, source, receiver).await
}

#[tokio::main]
async fn run_lab(args: &[String]) -> Result<(), Error> {
    let config = lab::parse(args)?;
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(async move {
        watch_signals(sender).await;
    });
    let report = lab::run(config, receiver).await?;
    print!("{report}");
    Ok(())
}

async fn watch_signals(sender: watch::Sender<bool>) {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                eprintln!("config: {error}");
                let _ = sender.send(true);
                return;
            }
        };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                eprintln!("config: {error}");
            }
        }
        _ = terminate.recv() => {}
    }
    let _ = sender.send(true);
}
