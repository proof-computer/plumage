# plumage development

plumage is the colour swarm, the first consumer of the `forthebirds` library.
The workspace root AGENTS.md of the `liskov` orchestrator governs worktrees,
documentation and delivery; this file governs local conventions.

## Layout

- `Cargo.toml`: one application package building the `plumage` binary; commit
  `Cargo.lock`.
- `src/lib.rs`: the library the binary, tests and lab use; every module is
  declared here.
- `src/main.rs`: the entrypoint. Keep it small as functionality lands in
  focused modules.
- `src/launch.rs`: `plumage launch`, the harness call. It does not supervise the peer.
- `src/liskov.rs`: Liskov-mode composition. The peer reads the helper directory,
  serves the site and reports readiness. It does not restart.
- `src/serve.rs`: the read-only site server.
- `src/lab.rs` and `tests/lab.rs`: the workstation lab and its offline whole-day test.
- `tests/liskov.rs`: the offline Liskov-mode tests. `tests/fixtures/` holds the
  helper directory files.
- `rust-toolchain.toml`: the pinned development and CI toolchain.
- `.github/workflows/ci.yml`: the local checks repeated on GitHub, on x86_64
  and native AArch64. It also compiles the payload scripts and runs
  `scripts/test-package-acurast.py`.
- `.github/workflows/plumage-runtime-image.yml`: dispatch-only image build.
  It does not publish the policy.
- `.github/workflows/plumage-payload.yml`: dispatch-only payload build and
  no-spend pin.
- `scripts/`: the payload packer, its test, the inference-server build and
  the V5 release filler.
- `.liskov/`: the source manifest, the unpublished policy draft and the
  launcher field allowlist.

## Rules

- Generic machinery belongs in `forthebirds`. This repository holds only the
  colour domain: its actions (`fetch`, `argue`, `vote`, and any decided
  companions), its source allowlist, its closing rule, its templates and its
  Liskov policy.
- The closing rule is a deterministic function over replicated records. It is
  published on the site next to the result. It never breaks a tie the swarm did
  not make.
- The site shows exact declarations and published formulas. It never labels a
  group a faction or a message persuasion.
- Fetched web text is data. Bound it, digest it, store it as evidence, and
  never let it reach a prompt outside a runtime-owned template.
- `unsafe_code` is forbidden.

## Validation

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release --bin plumage
./target/release/plumage
```

## Landing on `main`

Push the agent branch, open the pull request, arm
`gh pr merge <n> --merge --auto`, and move on; do not poll the checks.
