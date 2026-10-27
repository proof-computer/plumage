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
- `src/lab.rs` and `tests/lab.rs`: the workstation lab and its offline whole-day test.
- `rust-toolchain.toml`: the pinned development and CI toolchain.
- `.github/workflows/ci.yml`: the local checks repeated on GitHub, on x86_64
  and native AArch64.

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
