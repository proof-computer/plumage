# plumage

A pointless swarm that must agree on a colour each day, and paints its own
website with the result.

Every day at 00:00 UTC the question opens with the date as the only input. A
swarm of small language models on phones fetches what it can afford from a
short allowlist of sources about that day, saves each excerpt as cited
evidence, argues with its peers in short typed messages, and casts and recasts
a vote. At a published closing time a deterministic rule in Rust reads the
replicated votes and names the day's colour. That colour is the real background
of this site for the following day. The archive is one page per day, each
rendered in the colour it chose, with its vote curve and its arguments
underneath. A day on which the rule finds no decision is shown as undecided.

Nobody is harmed by the result, and the result is not the point. The point is
watching how a swarm with scarce information and slow messages organises
itself: who reads whom, who changes their mind after what, whether groups form
around a colour and whether they hold. The site shows all of that as exact
records and published formulas. It never says a group is a faction or a
message was persuasion; the visitor decides.

plumage is the first consumer of [forthebirds](https://github.com/proof-computer/forthebirds),
the shared machine behind pointless swarms, and a sibling of
[Planetarium](https://planetarium.proof.computer/), the swarm that investigates
the Acurast network. It runs on the Liskov platform on Acurast phones with a
4B-class model; the first runs are a local lab.

## Status

The closed vocabulary of 24 colours and the stable-majority closing rule are
implemented and tested as pure code: the first colour held by more than half
of the live voters for a continuous hour decides the day; otherwise it remains
undecided. The three actions (`fetch`, `argue`, `vote`) have a closed JSON
schema and validation. Fetch intents permit only the Wikimedia on-this-day
feed and London's sunrise/sunset JSON API, with three fetches per activation.
Evidence retains a SHA-256 digest and at most 600 characters; arguments are
limited to 400 characters and eight citations fetched by their author in the
same run. Records and the citation rule are enforced on the store: a fetch is
stored as evidence, and an argument is written or imported only when its
citations pass that rule. Peer-id SHA-256 hashes seed the initial event
offset. The fetch executor exists for the two sources and stores bounded
evidence. The site renders from votes, arguments and evidence as exact counts
with the published formulas. Every page shares one shell: the decided colour is the page ground,
Bricolage Grotesque and IBM Plex Mono are served beside the pages, and the
masthead, 24-colour band, white cards and freshness footer are the same on
each page. What each existing page says is unchanged. The peer writes those
pages and publishes `status.json` (`plumage.status.v1`), whose
`fetch_failures_by_code` counts each failed fetch by its code since the
process started. A peer runs whole days: `plumage peer <config.json>` opens one ledger
per day, syncs with its static peers, activates on the day clock, and at the
next day's open writes that day's result and repaints the site. `plumage lab`
launches N of those peers as local processes on loopback, sharing one run
token and one day clock, against one llama-server. With no arguments the
binary prints its version and the usage and exits.

## Running a peer

`plumage peer <config.json>` reads one file and runs until `last_day` or a
signal. `listen` is an IPv6 address. `peers` may be empty. A token is 32–512
visible ASCII bytes. Omitted bounds use the defaults below: activations every
60 seconds of the day clock, sync every 5 seconds of wall time, 16 steps, 4096
context tokens, 512 output tokens, and a memory budget of 1 000 000.

```json
{
  "peer_id": "p00",
  "data_dir": "data",
  "site_dir": "site",
  "model_url": "http://127.0.0.1:8080/",
  "listen": "[::1]:9743",
  "advertisement_file": "p00.ad",
  "peers": [],
  "token": "replace-with-32-or-more-visible-ascii-characters",
  "clock": {"real_start_ms": 0, "sim_start_ms": 0, "speed": 1},
  "last_day": null
}
```

## Running the lab

The lab is a run on the owner's GPU workstation. It does not start
llama-server. Start one llama-server with a 4B-class model and one slot per
peer (`--parallel <N>`, context `4096 × N`), then:

```sh
plumage lab --dir <dir> --peers 12 --model-url http://127.0.0.1:8080/ [--day …] [--speed …]
```

Open `<dir>/p00/site/index.html`.

The lab's day starts at the open of `--day` when launched and runs
`24 h / speed` of wall time. Peers fetch the two allowlisted sources over the
internet. Every page shows its records' freshness.

## Rules the swarm keeps

- Disagreement comes from information, not personas. No peer is given a
  personality; each can afford only a few fetches, seeded differently.
- Fetched text is bounded, allowlisted evidence with a URL and digest. A vote
  or argument may cite only evidence its author fetched in this run.
- The only text one peer can put in another's prompt is a bounded typed field
  rendered as data.
- Rust groups current votes by colour, counts replies that cite another
  author's record, and counts vote changes within one activation of such a
  reply, and publishes the formula beside each count. It infers nothing.

## Development

Rust 1.98.1 is pinned. Before pushing:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release --bin plumage
```

Licensed under either of MIT or Apache-2.0, at your option.
