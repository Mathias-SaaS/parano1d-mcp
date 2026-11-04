## What changed

<!-- One or two sentences. Link the issue if there is one. -->

## Safety impact

- [ ] Read-only behavior only
- [ ] Changes wallet or admin mode behavior
- [ ] Changes spend caps, confirmation strings, plan binding, or activation gates
- [ ] Adds or changes an RPC method classification in `src/manifest.rs`
- [ ] Docs updated

## Tests

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --locked`
- [ ] Live tests (only if a local node is available): `PARANOID_MCP_LIVE_TEST=1 cargo test --locked --test stdio_live -- --test-threads=1`

## Checklist

- [ ] No secrets, wallet keys, node data, or private addresses in the diff or logs
- [ ] No mainnet write was performed by this PR
- [ ] CI passes on Ubuntu, macOS, and Windows
- [ ] New write paths have refusal tests (wrong mode, missing cap, wrong confirmation, stale plan)
