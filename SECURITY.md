# Security policy

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | Yes, read-only mode. Wallet and admin modes are opt-in; admin mode and contract/native writes have not been exercised on mainnet. |

## Reporting a vulnerability

Do not open a public issue. Use GitHub's private vulnerability reporting:
**Security → Advisories → Report a vulnerability**, or open
<https://github.com/0xC0rd31r0/parano1d-mcp/security/advisories/new> directly (private
vulnerability reporting is enabled on this repository).

Include the affected version, platform, mode (`read`, `wallet`, `admin`), reproduction steps,
and impact. Reports are acknowledged as soon as practical, normally within 7 days, and triaged
within 14. This project has no bug bounty.

## Scope

In scope:

- Bypassing a mode gate (`read` to `wallet` or `admin`).
- Bypassing or exceeding `PARANOID_MCP_MAX_SEND_MICRONOID` or `PARANOID_MCP_MAX_FEE_MICRONOID`.
- Reusing or forging a send, consolidation, or native `plan_id`; bypassing a confirmation string.
- Bypassing the loopback-only RPC check through URL parsing or redirects.
- A panic, unbounded allocation, or hang reachable from node responses or tool input.
- A tool description or schema that misrepresents what the tool does.

Out of scope:

- The node's RPC being unauthenticated. That is a node design property, documented in the README.
- A local process or user with access to the node's RPC; it already has owner scope.
- MCP client auto-approval, prompt injection in third-party clients, or model behaviour.
- Vulnerabilities in the Parano1d node itself; report those upstream.
- Losses caused by enabling wallet mode with a cap you chose.
- Missing TLS; the RPC is loopback-only by design.

## Threat model

The server assumes a single-user machine, a trusted node binary, and a local RPC endpoint that any
local process can reach. It guards against accidental and agent-driven spends, not against a local
attacker or a compromised client. See "Trust boundary" in the README.

## Cancellation

An MCP client can cancel a tool call, which drops the server's future at any `.await`. The design
keeps that safe, and the limits are known:

- The wallet lock is a `tokio::sync::Mutex`; its guard is released when the cancelled call is
  dropped, so a later wallet call cannot deadlock.
- A send plan is consumed before the signing RPC is sent, so a cancelled call can never reuse it;
  the plan is spent even though nothing (or everything) happened.
- If the cancellation happens **after** the `walletSend` request was sent, the outcome is unknown
  in exactly the same way as a lost response: the node may have accepted the transaction. Treat it
  as "check `getTx` and the wallet history", never as "nothing happened", and never resend blindly.
- Cancellation before the request is sent leaves no half-state: the plan is consumed, the lock is
  released, and no RPC was made.

## Disclosure

90 days, or sooner by agreement. Credit is given unless you ask otherwise.
