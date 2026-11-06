# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.5] - 2026-11-06

### Added

- README: "Get a Parano1d node" — the desktop app from parano1d.org (Downloads), the official
  Core release archives for headless use, the start command for an ordinary node with RPC on
  `127.0.0.1:9601`, and what a node needs (CPU, memory, disk, sync, network).

### Changed

- When no node answers at the RPC URL (connection refused, or the connect timed out), `--check`
  and every tool now say so and where to get one: "No Parano1d node answered at <url>. Install and
  start one: https://parano1d.org (Downloads), then retry." The reason follows in brackets and the
  text stays bounded. `--check` reports it once (not once per probe call) and also prints it on
  stderr. Structured errors use the kind `node_unreachable`.
- Connecting to the node times out after 5 seconds. A failed connect sent nothing, so a write
  that could not connect is reported as a definite failure, not as an unknown outcome; a
  connection that opened and then timed out is still an unknown outcome and is never retried.
- The release workflow no longer marks a plain version (`v0.1.5`) as a pre-release, so it can
  become the latest release when its draft is published; a tag with a SemVer pre-release suffix
  (`-rc.1`, `-beta`) is still marked pre-release. The release is still created as a draft.

## [0.1.4] - 2026-11-05

No change to the server's behaviour, tools or guards.

### Added

- Install from crates.io: `cargo install parano1d-mcp --locked`.
- A `server.json` for the official MCP Registry, as `io.github.noidium/parano1d-mcp`: the server
  is listed as read-only by default, with sending disabled unless the user explicitly enables the
  capped wallet mode.
- The release workflow publishes the tagged version to crates.io (trusted publishing) and then to
  the MCP Registry (GitHub OIDC), after the build, attestation and draft-release jobs.
- Issue forms ask for the node version, the network, the tool and the JSON-RPC method involved,
  and (optionally) whether an AI agent filed the request.

### Changed

- The repository moved to [github.com/noidium/parano1d-mcp](https://github.com/noidium/parano1d-mcp);
  the old URL redirects.
- CI's secret scan runs the pinned gitleaks CLI instead of gitleaks-action, which needs a
  licence key in an organisation.
- `CODEOWNERS`, `SECURITY.md` and the issue forms point to the new repository and maintainer.

## [0.1.3] - 2026-10-29

### Security

- Native trades are capped at what they authorise, not only at their quote. A trade signs a
  `gross_input` and a `fee_limit`, and the block it lands in decides the fill, so `native_write`
  now caps a buy at `gross_input + fee_limit` and a sell at `fee_limit` (or the quote, if larger),
  and the optional fee cap applies to `fee_limit`. A trade preview without `fee_limit_micronoid`
  is refused. Other native operations keep their exact debit-plus-fee cap.
- `contract_write call` previews the identical request first and caps the previewed payout plus
  fee. A closing (`terminal`) call carries no payout field but releases the whole contract
  balance to the recipient its terms commit, so it was previously checked against its fee only.

### Changed

- `PARANOID_MCP_MAX_RESULT_BYTES` applies to the text actually returned (pretty JSON); results
  are serialized once.
- The node's JSON-RPC error message is bounded to 2,000 characters, like its error data.
- At most 64 unredeemed plans of each kind (send, consolidation, native) are kept; the oldest is
  dropped first.
- `mining submit_block` requires `nonce_hex` to be 32 lowercase hex characters, as its schema
  says and the node requires.
- A capability probe that failed (for example, the node was briefly down) is reported as a
  failed probe instead of as a node without the capability, and is not cached.
- The README states which guard each write has.

## [0.1.2] - 2026-10-29

The first release: an MCP server for a local Parano1d node, read-only by default, with the whole
RPC surface mapped to 18 tools and every spend behind a plan, a confirmation and a cap.

### Added

- 18 MCP tools over stdio: node overview, chain, mempool, fees, address validation, wallet reads
  and send planning, guarded sends, signed-intent submission, wallet maintenance, contract reads
  and writes, native v2.1 reads and writes, peer report, mining, node control, and a read-only RPC
  passthrough.
- Runtime detection of node capabilities through `getNativeProtocol` and `getContractProtocol`;
  support for node v2.0.4 (68 RPC methods) and v2.1.0 (79 methods).
- `--check`, `--version` and `--help` CLI entry points; `--check` probes the node and prints its
  capabilities without starting the MCP server.
- Build-provenance attestations for every release archive, `cargo auditable` binaries, and a
  `SHA256SUMS` file.
- CI on Ubuntu, macOS and Windows: formatting, clippy, tests and a release build, plus a
  `cargo-deny` job and an `audit` job (`zizmor`, `actionlint`, `gitleaks`).
- A hostile JSON-RPC test suite that runs without a node: oversized and endless streams, lying
  content-lengths, invalid UTF-8/JSON, deep nesting, response-id mismatch, batch-id edge cases,
  redirects, 4xx/5xx, stalled and closed connections, unknown fields, plan source/quote/network
  changes, a cancelled send, and every write refusal.

### Platforms

- macOS arm64/x86_64, Linux x86_64/arm64 and Windows x86_64. No TLS stack, no OpenSSL, no CMake,
  no NASM, no listening socket, no admin rights.

### Security

- Read-only mode is the default. Wallet and admin modes are explicit opt-in, and every write is
  checked against the mode before any RPC.
- Loopback-only RPC URL validation; hostnames, credentials in the URL and remote hosts are
  refused. The HTTP client ignores proxy environment variables and refuses redirects.
- One-time, 120-second send plans bound to the source key index and address, the node's network,
  the destination, the amount and the fee, with an exact confirmation string. A changed source,
  quote or network refuses the send. Wallet-signing operations are serialized in-process.
- Per-call spend caps cover amount + fee (native: debit + fee); the optional fee cap applies to
  the node's quoted fee as well. Contract funding and calls require an explicit non-zero fee.
  Writes are never retried, and a write whose response is lost is reported as **outcome unknown**
  rather than as a failure.
- Tool argument objects reject unknown fields (`additionalProperties: false`).
- Native v2.1 writes are gated on the node's activation state; `set_active_address` requires
  `SET ACTIVE ADDRESS <index>`; `stop` requires `STOP AT HEIGHT <current height>`.
- Request and response sizes are bounded, responses are streamed under a hard cap, and single
  responses must match the request id.
- Release builds enable `overflow-checks` and are byte-reproducible on the same toolchain.
