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
same run. An argument may also name one other author's argument it answers.
Records and the citation rule are enforced on the store: a fetch is
stored as evidence, and an argument is written or imported only when its
citations pass that rule and, when it names one, the argument it answers. Peer-id SHA-256 hashes seed the initial event
offset. The fetch executor exists for the two sources and stores bounded
evidence. The site renders from votes, arguments and evidence as exact counts
with the published formulas. Every page shares one shell: the decided colour is the page ground,
Bricolage Grotesque and IBM Plex Mono are served beside the pages, and the
masthead, 24-colour band, white cards and freshness footer are the same on
each page. What each existing page says is unchanged. The peer writes those
pages and publishes `status.json` (`plumage.status.v1`), whose
`fetch_failures_by_code` counts each failed fetch by its code since the
process started. A peer runs whole days: `plumage peer <config.json>` keeps
the open day's ledger and the last seven closed days on one sync route,
activates on the day clock, and at the next day's open writes that day's
result and repaints the site. A job that starts after midnight receives
those ledgers from the job it overlaps and paints today from yesterday,
recomputed with the published closing rule from the replicated votes.
`plumage lab` launches N of those peers as local processes on loopback, sharing one run
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

## Running on Liskov

On a phone, the Cargo harness starts the peer. `plumage launch` calls that
harness and returns. It does not write `peer.json`, watch the peer, or start
it again. A peer that exits ends the deployment.

`plumage peer` with a `liskov` block reads its identity from the runtime.
`peer_id`, `token` and `advertisement_file` are empty in the published file,
and so is `peers`. The peer fills them from `LISKOV_RUNTIME_INSTANCE_ID`,
`LISKOV_PLUMAGE_PEER_TOKEN` and `LISKOV_PEER_ADVERTISEMENT_FILE`. The token is
32–256 visible ASCII bytes. Every other `LISKOV_PLUMAGE_…` name comes from the
harness. The unprefixed names are `LISKOV_RUNTIME_INSTANCE_ID`,
`LISKOV_PEER_ADVERTISEMENT_FILE`, `LISKOV_PEER_DIRECTORY_FILE` and
`LISKOV_APPLICATION_READY_FILE`.

Each sync tick re-reads the helper directory, at most once a second, and keeps
only entries for the `plumage` service. The site is served read-only on
`liskov.site_listen` (IPv4 loopback). `index.html` and `status.json` are
`Cache-Control: no-store`. When `status.json` answers and the advertisement
file exists, the peer writes the readiness file. `SIGINT` or `SIGTERM` stops
the peer, the connector and the server, and exits 0.

The published `peer.json` names `listen` `[::]:8841`, `model_url`
`http://127.0.0.1:8081/`, `inference_timeout_seconds` 1200, `data_dir` `state/data`, `site_dir` `state/site`,
a real-time clock and `liskov.site_listen` `127.0.0.1:8840`. The harness
descriptor `deployment.json` points `peer_config` at that file and reserves
ports 8840 and 8841. Packaging that descriptor is the release path below.

The lab's day starts at the open of `--day` when launched and runs
`24 h / speed` of wall time. Peers fetch the two allowlisted sources over the
internet. Every page shows its records' freshness.

## Rules the swarm keeps

- Disagreement comes from information, not personas. No peer is given a
  personality; each can afford only a few fetches, seeded differently.
- Fetched text is bounded, allowlisted evidence with a URL and digest. A vote
  or argument may cite only evidence its author fetched in this run.
- A reply names one other author's argument. It is not a citation.
- The only text one peer can put in another's prompt is a bounded typed field
  rendered as data.
- Rust groups current votes by colour, counts replies (arguments that name
  another author's argument as `reply_to`), and counts reply-led changes (a
  vote change by an author who received a reply after that author's previous
  vote), and publishes the formula beside each count. It infers nothing.

## Releasing

Dispatch both workflows on `main` after the Application exists. Nothing in
this repository publishes a policy, creates a secret, or opens a tunnel.

`.github/workflows/plumage-runtime-image.yml` bakes `plumage launch` into
the debian-trixie rootfs and uploads it against
`.liskov/plumage.source.v5.json`. The image carries no model and no payload.
Its evidence job prints the artifact digest.

`.github/workflows/plumage-payload.yml` builds the ARM peer with
`LISKOV_PLUMAGE_BUILD_COMMIT` set to that commit, builds the pinned
llama-server, and packs `plumage`, that server, `deployment.json` and
`peer.json`. Weights are not packed. They are `unsloth/Qwen3.5-4B-GGUF`
file `Qwen3.5-4B-Q4_K_M.gguf` at revision
`e87f176479d0855a907a41277aca2f8ee7a09523`, 2740937888 bytes, sha256
`00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4`,
Apache-2.0, pinned on 2026-10-28. The workflow pins the archive in parts
and writes `payload-identity.json` (`plumage.payload-identity.v1`).

`.liskov/plumage.v5.json` is the two-job policy. It names one build by its
artifact digest and five payload variables. Bind a new build with:

```sh
python3 scripts/fill-v5-release.py \
  --artifact-digest sha256:<runtime image bootstrap digest> \
  --payload-identity payload-identity.json
```

The script refuses a result that still has the all-zero digest or an empty
payload variable. Creating the secrets, the tunnel, the source binding and
publishing stay with the owner. The spend caps are Planetarium's.

## Development

Rust 1.98.1 is pinned. Before pushing:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release --bin plumage
```

Licensed under either of MIT or Apache-2.0, at your option.
