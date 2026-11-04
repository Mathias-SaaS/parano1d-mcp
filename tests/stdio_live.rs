// SPDX-License-Identifier: Apache-2.0
//! Live read-only MCP tests. They require the local node and are skipped unless
//! `PARANOID_MCP_LIVE_TEST=1` is set.
//!
//! Nothing here submits a transaction, changes the active address, stops the
//! node or writes chain state. The same-index set_active case exercises the
//! confirmation path, but the node returns the existing active address without
//! changing it.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn spawn(mode: &str, url: Option<&str>, extra: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parano1d-mcp"));
        command
            .env("PARANOID_MCP_MODE", mode)
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.env("PARANOID_RPC_URL", url.unwrap_or("http://127.0.0.1:9601"));
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn parano1d-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut server = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "parano1d-mcp-test", "version": "0"}
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "parano1d-mcp");
        server.notify("notifications/initialized", json!({}));
        server
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").expect("write MCP message");
        self.stdin.flush().expect("flush MCP message");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read MCP message");
            assert!(read > 0, "MCP server closed stdout");
            let value: Value = serde_json::from_str(&line).expect("valid MCP JSON");
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
    }

    fn text(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.call(tool, arguments);
        assert!(
            response.get("error").is_none(),
            "{tool} transport error: {response}"
        );
        assert_ne!(
            response["result"]["isError"].as_bool(),
            Some(true),
            "{tool} returned an error: {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        serde_json::from_str(text).unwrap_or_else(|_| json!(text))
    }

    fn expect_error(&mut self, tool: &str, arguments: Value) -> String {
        let response = self.call(tool, arguments);
        if let Some(error) = response.get("error") {
            return error.to_string();
        }
        assert_eq!(
            response["result"]["isError"].as_bool(),
            Some(true),
            "{tool} should have failed: {response}"
        );
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("error text")
            .to_string()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn live_enabled() -> bool {
    std::env::var("PARANOID_MCP_LIVE_TEST").ok().as_deref() == Some("1")
}

fn live_url() -> String {
    std::env::var("PARANOID_MCP_LIVE_URL").unwrap_or_else(|_| "http://127.0.0.1:9601".into())
}

fn live_testnet_url() -> String {
    std::env::var("PARANOID_MCP_LIVE_TESTNET_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:19821".into())
}

#[test]
fn live_mainnet_read_only() {
    if !live_enabled() {
        eprintln!("skipping live test: set PARANOID_MCP_LIVE_TEST=1");
        return;
    }
    let mut mcp = Mcp::spawn("read", Some(&live_url()), &[]);

    let overview = mcp.text("node_overview", json!({}));
    assert_eq!(overview["capabilities"]["detected"], "2.1.x");
    assert_eq!(overview["mode"], "read");
    let height = overview["chain"]["height"].as_u64().expect("tip height");
    assert!(height > 0);
    let activation = overview["capabilities"]["native_activation_height"]
        .as_u64()
        .expect("native activation height");
    let native_active = overview["capabilities"]["native_active"]
        .as_bool()
        .expect("native active flag");
    assert!(activation > 0);
    // The node may flip active_at_next_block one block before activation, so
    // only assert outside the boundary block.
    if height + 1 < activation {
        assert!(!native_active);
    }
    if height >= activation {
        assert!(native_active);
    }

    let wallet = mcp.text("wallet_read", json!({"action": "status"}));
    let address = wallet["address"].as_str().expect("wallet address");
    let active_index = wallet["active_index"].as_u64().expect("active index");
    assert_eq!(wallet["exists"], true);

    let plan = mcp.text(
        "wallet_plan_send",
        json!({"to": address, "amount_micronoid": 100_000, "fee_micronoid": 0}),
    );
    assert!(plan["plan_id"].is_string());
    assert!(plan["plan"]["fee_micronoid"].as_u64().unwrap() > 0);
    assert_eq!(plan["from_index"].as_u64(), Some(active_index));
    assert_eq!(plan["from_address"].as_str(), Some(address));

    let inactive_index = active_index + 1;
    let refused = mcp.expect_error(
        "wallet_plan_send",
        json!({
            "to": address,
            "amount_micronoid": 1000,
            "fee_micronoid": 0,
            "from_index": inactive_index
        }),
    );
    assert!(refused.contains("not active"), "{refused}");
    assert!(
        refused.contains(&format!("SET ACTIVE ADDRESS {inactive_index}")),
        "{refused}"
    );

    let receipts = mcp.text(
        "wallet_read",
        json!({"action": "query_receipts", "query": {}, "page": 1, "page_size": 2}),
    );
    assert!(receipts["total"].is_number());

    let native = mcp.text("native_read", json!({"action": "protocol"}));
    let native_activation = native["activation_height"]
        .as_u64()
        .expect("native activation height");
    let native_tip = native["tip_height"].as_u64().expect("native tip height");
    let active_next = native["active_at_next_block"]
        .as_bool()
        .expect("native active_at_next_block");
    assert!(native_activation > 0);
    if native_tip + 1 < native_activation {
        assert!(!active_next);
    }
    if native_tip >= native_activation {
        assert!(active_next);
    }

    let contracts = mcp.text("contract_read", json!({"action": "protocol"}));
    assert_eq!(contracts["runtime_available"], true);

    let header = mcp.text(
        "chain_query",
        json!({"action": "block_header", "height": height.saturating_sub(1)}),
    );
    assert!(header["hash"].is_string());

    let block = mcp.text(
        "chain_query",
        json!({"action": "block", "height": height.saturating_sub(2), "include_raw": false}),
    );
    assert_eq!(block["raw_omitted"], true);

    let mempool = mcp.text("mempool_query", json!({"action": "stats"}));
    assert!(mempool["capacity"].is_number());

    let fee = mcp.text("fee_estimate", json!({"n_outputs": 2}));
    assert!(fee.is_number() || fee["fee_micronoid"].is_number());

    let mining = mcp.text("mining", json!({"action": "info"}));
    assert!(mining["height"].is_number());

    let peers = mcp.text("peer_report", json!({}));
    assert!(peers["peer_count"].is_number());
    assert!(peers["summary"]["connected_peers"].is_number());
    assert_eq!(peers["per_peer_details"]["available"], false);
    assert_eq!(peers["per_peer_details"]["historical_cache_enabled"], false);

    let refused = mcp.expect_error(
        "rpc_call",
        json!({"method": "walletSend", "params": [address, 1000, 0]}),
    );
    assert!(refused.contains("write/control"));

    let refused = mcp.expect_error(
        "node_control",
        json!({"action": "stop", "confirm": "STOP AT HEIGHT 0"}),
    );
    assert!(refused.contains("requires mode admin"));

    let refused = mcp.expect_error(
        "contract_write",
        json!({"action": "watch", "opening_hex": "00"}),
    );
    assert!(refused.contains("requires mode wallet"));

    let refused = mcp.expect_error(
        "tx_submit",
        json!({"intent_hex": "00", "confirm": "SUBMIT SIGNED INTENT"}),
    );
    assert!(refused.contains("requires mode wallet"));

    let refused = mcp.expect_error(
        "rpc_call",
        json!({"method": "submitTxIntent", "params": ["00"]}),
    );
    assert!(refused.contains("write/control"));
}

#[test]
fn live_wallet_mode_safety_without_spending() {
    if !live_enabled() {
        eprintln!("skipping live test: set PARANOID_MCP_LIVE_TEST=1");
        return;
    }

    let mut no_cap = Mcp::spawn("wallet", Some(&live_url()), &[]);
    let error = no_cap.expect_error(
        "wallet_send",
        json!({
            "to": "o1test",
            "amount_micronoid": 1,
            "plan_id": "missing",
            "confirm": "SEND 1 MICRONOID TO o1test"
        }),
    );
    assert!(error.contains("PARANOID_MCP_MAX_SEND_MICRONOID"), "{error}");

    let mut capped = Mcp::spawn(
        "wallet",
        Some(&live_url()),
        &[("PARANOID_MCP_MAX_SEND_MICRONOID", "1000")],
    );
    let error = capped.expect_error(
        "wallet_send",
        json!({
            "to": "o1test",
            "amount_micronoid": 1,
            "plan_id": "missing",
            "confirm": "SEND 1 MICRONOID TO o1test"
        }),
    );
    assert!(error.contains("unknown or already used"), "{error}");

    let error = capped.expect_error(
        "native_write",
        json!({
            "action": "submit",
            "request": {"action": "burn_token", "asset_id": "00", "units": "1"},
            "plan_id": "missing"
        }),
    );
    assert!(
        error.contains("not active yet")
            || error.contains("activation")
            || error.contains("unknown or already used native plan_id"),
        "{error}"
    );

    let error = capped.expect_error(
        "tx_submit",
        json!({"intent_hex": "00", "confirm": "SUBMIT SIGNED INTENT"}),
    );
    assert!(error.contains("PARANOID_MCP_ALLOW_RAW_INTENT"), "{error}");

    let mut source = Mcp::spawn(
        "wallet",
        Some(&live_url()),
        &[
            ("PARANOID_MCP_MAX_SEND_MICRONOID", "100000"),
            ("PARANOID_MCP_MAX_FEE_MICRONOID", "50000"),
        ],
    );
    let status = source.text("wallet_read", json!({"action": "status"}));
    let address = status["address"].as_str().expect("wallet address");
    let active_index = status["active_index"].as_u64().expect("active index");
    let plan = source.text(
        "wallet_plan_send",
        json!({"to": address, "amount_micronoid": 1000, "fee_micronoid": 0}),
    );
    let plan_id = plan["plan_id"].as_str().expect("plan id");
    assert_eq!(plan["from_index"].as_u64(), Some(active_index));
    assert_eq!(plan["from_address"].as_str(), Some(address));

    let error = source.expect_error(
        "wallet_send",
        json!({
            "to": address,
            "amount_micronoid": 1000,
            "fee_micronoid": 0,
            "from_index": active_index + 1,
            "plan_id": plan_id,
            "confirm": format!("SEND 1000 MICRONOID TO {address}")
        }),
    );
    assert!(error.contains("belongs to source index"), "{error}");

    let error = source.expect_error(
        "wallet_maintenance",
        json!({"action": "set_active_address", "index": active_index, "confirm": "WRONG"}),
    );
    assert!(error.contains("confirmation mismatch"), "{error}");
    let error = source.expect_error(
        "wallet_maintenance",
        json!({"action": "set_active_address", "index": active_index}),
    );
    assert!(error.contains("confirmation mismatch"), "{error}");

    let same = source.text(
        "wallet_maintenance",
        json!({
            "action": "set_active_address",
            "index": active_index,
            "confirm": format!("SET ACTIVE ADDRESS {active_index}")
        }),
    );
    assert_eq!(same["key_index"].as_u64(), Some(active_index));
    let after = source.text("wallet_read", json!({"action": "status"}));
    assert_eq!(after["active_index"].as_u64(), Some(active_index));
    assert_eq!(after["address"].as_str(), Some(address));

    let mut admin = Mcp::spawn("admin", Some(&live_url()), &[]);
    let error = admin.expect_error(
        "node_control",
        json!({"action": "stop", "confirm": "STOP AT HEIGHT 0"}),
    );
    assert!(error.contains("confirmation mismatch"), "{error}");
}

#[test]
fn live_testnet_native_read() {
    if !live_enabled() {
        eprintln!("skipping live test: set PARANOID_MCP_LIVE_TEST=1");
        return;
    }
    let mut mcp = Mcp::spawn("read", Some(&live_testnet_url()), &[]);
    let native = mcp.text("native_read", json!({"action": "protocol"}));
    assert_eq!(native["active_at_next_block"], true);
    let markets = mcp.text(
        "native_read",
        json!({"action": "catalog", "kind": "markets", "limit": 2}),
    );
    assert!(markets["items"].is_array());
}
