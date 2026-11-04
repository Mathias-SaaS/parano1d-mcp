# Contributing

## Before you start

Open an issue for anything larger than a bug fix. For security issues, read `SECURITY.md` and
report privately.

## Build and test

Rust 1.96 is pinned by `rust-toolchain.toml`.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

Live tests need a v2.1 node with a wallet and are opt-in:

```sh
PARANOID_MCP_LIVE_TEST=1 cargo test --locked --test stdio_live -- --test-threads=1
```

## Code rules

- Read-only is the default. A new tool that can spend must be classified in `src/manifest.rs` and
  gated by mode.
- Every write path needs refusal tests: wrong mode, missing cap, wrong confirmation, stale plan.
- Do not add a TLS stack, a network listener, filesystem access to node files, or telemetry.
- Do not add a dependency without a reason. The dependency tree is part of the attack surface.
- Keep the binary portable: no OS-specific code, no shell-outs, no admin rights.
- A new RPC method is a manifest entry plus a tool action, not a new ad-hoc HTTP call.

## Commits and pull requests

- One logical change per PR. Imperative commit subject.
- CI must pass on Ubuntu, macOS, and Windows.
- Describe the safety impact: which modes, caps, confirmations, or activation gates change.
- By submitting a pull request you agree to license your contribution under Apache-2.0.
