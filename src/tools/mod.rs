// SPDX-License-Identifier: Apache-2.0
//! MCP tools. Read-only is the default; every write is mode-gated in code.

pub mod chain;
pub mod contracts;
pub mod mining;
pub mod native;
pub mod peers;
pub mod wallet;

use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::capabilities::CapabilityCache;
use crate::config::Config;
use crate::manifest::{self, Access, MethodSpec};
use crate::rpc::{RpcClient, RpcError};
use crate::safety::PlanStore;

use self::chain::{AddressValidateArgs, ChainQueryArgs, FeeEstimateArgs, MempoolQueryArgs};
use self::contracts::{ContractReadArgs, ContractWriteArgs};
use self::mining::{MiningArgs, NodeControlArgs};
use self::native::{NativeReadArgs, NativeWriteArgs};
use self::wallet::{
    TxSubmitArgs, WalletMaintenanceArgs, WalletPlanSendArgs, WalletReadArgs, WalletSendArgs,
};

#[derive(Clone)]
pub struct Parano1dServer {
    pub(crate) cfg: Arc<Config>,
    pub(crate) rpc: Arc<RpcClient>,
    pub(crate) caps: Arc<CapabilityCache>,
    pub(crate) plans: Arc<PlanStore>,
    /// Serializes wallet operations inside this process. The node's wallet
    /// signs one operation at a time, but a concurrent set_active_address must
    /// not slip between a send's source check and its walletSend call.
    pub(crate) wallet_ops: Arc<tokio::sync::Mutex<()>>,
    tool_router: ToolRouter<Self>,
}

impl Parano1dServer {
    pub fn new(config: Config) -> Result<Self, String> {
        let rpc = Arc::new(RpcClient::new(config.rpc_url.clone())?);
        let caps = Arc::new(CapabilityCache::new(rpc.clone()));
        Ok(Self {
            cfg: Arc::new(config),
            rpc,
            caps,
            plans: Arc::new(PlanStore::new()),
            wallet_ops: Arc::new(tokio::sync::Mutex::new(())),
            tool_router: Self::tool_router(),
        })
    }
}

#[tool_router]
impl Parano1dServer {
    #[tool(
        description = "One call with chain tip, node/P2P status, mining, mempool, contract/native capabilities and wallet status. Read-only.",
        annotations(
            title = "Node overview",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn node_overview(&self) -> Result<String, String> {
        self.node_overview_impl().await
    }

    #[tool(
        description = "Chain, block, header, slot, state, transaction and receipt queries. Read-only.",
        annotations(
            title = "Chain query",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn chain_query(
        &self,
        Parameters(args): Parameters<ChainQueryArgs>,
    ) -> Result<String, String> {
        self.chain_query_impl(args).await
    }

    #[tool(
        description = "Mempool info, size, stats or one pending entry. Read-only.",
        annotations(
            title = "Mempool query",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn mempool_query(
        &self,
        Parameters(args): Parameters<MempoolQueryArgs>,
    ) -> Result<String, String> {
        self.mempool_query_impl(args).await
    }

    #[tool(
        description = "Estimate the fee in μNOID for N inputs and M outputs. Read-only.",
        annotations(
            title = "Fee estimate",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn fee_estimate(
        &self,
        Parameters(args): Parameters<FeeEstimateArgs>,
    ) -> Result<String, String> {
        self.fee_estimate_impl(args).await
    }

    #[tool(
        description = "Validate and normalise an o1… address. Read-only.",
        annotations(
            title = "Validate address",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn address_validate(
        &self,
        Parameters(args): Parameters<AddressValidateArgs>,
    ) -> Result<String, String> {
        self.address_validate_impl(args).await
    }

    #[tool(
        description = "Wallet status, balance, addresses, UTXOs, history, saved receipts and mined blocks. Read-only.",
        annotations(
            title = "Wallet read",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wallet_read(
        &self,
        Parameters(args): Parameters<WalletReadArgs>,
    ) -> Result<String, String> {
        self.wallet_read_impl(args).await
    }

    #[tool(
        description = "Quote a NOID send without submitting it. The plan is bound to the active source address and returns a short-lived plan_id for wallet_send. Read-only.",
        annotations(
            title = "Plan NOID send",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wallet_plan_send(
        &self,
        Parameters(args): Parameters<WalletPlanSendArgs>,
    ) -> Result<String, String> {
        self.wallet_plan_send_impl(args).await
    }

    #[tool(
        description = "Send NOID from the active address. Requires PARANOID_MCP_MODE=wallet, a fresh plan_id from wallet_plan_send bound to that source, the exact confirm string, and PARANOID_MCP_MAX_SEND_MICRONOID. The node wallet signs; this server never sees a key.",
        annotations(
            title = "Send NOID",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn wallet_send(
        &self,
        Parameters(args): Parameters<WalletSendArgs>,
    ) -> Result<String, String> {
        self.wallet_send_impl(args).await
    }

    #[tool(
        description = "Submit a raw signed transaction intent (advanced). Requires mode wallet, PARANOID_MCP_ALLOW_RAW_INTENT=1 and confirm = SUBMIT SIGNED INTENT. The intent is already signed, so this path cannot apply the send-plan cap; it is separate from rpc_call for that reason.",
        annotations(
            title = "Submit signed intent",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn tx_submit(
        &self,
        Parameters(args): Parameters<TxSubmitArgs>,
    ) -> Result<String, String> {
        self.tx_submit_impl(args).await
    }

    #[tool(
        description = "Wallet rescan/discovery, address generation/switching, and UTXO consolidation. Requires mode wallet; set_active_address requires confirm = SET ACTIVE ADDRESS <index>; consolidation requires a plan_id from the plan action.",
        annotations(
            title = "Wallet maintenance",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn wallet_maintenance(
        &self,
        Parameters(args): Parameters<WalletMaintenanceArgs>,
    ) -> Result<String, String> {
        self.wallet_maintenance_impl(args).await
    }

    #[tool(
        description = "Contract protocol, instance status, call previews, retained states and receipts. Read-only.",
        annotations(
            title = "Contract read",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn contract_read(
        &self,
        Parameters(args): Parameters<ContractReadArgs>,
    ) -> Result<String, String> {
        self.contract_read_impl(args).await
    }

    #[tool(
        description = "Create, fund, call, watch or import contract objects. Requires mode wallet; funding and calls spend NOID.",
        annotations(
            title = "Contract write",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn contract_write(
        &self,
        Parameters(args): Parameters<ContractWriteArgs>,
    ) -> Result<String, String> {
        self.contract_write_impl(args).await
    }

    #[tool(
        description = "v2.1 native protocol, catalogues, markets, holdings, pending activity and operation previews. Reads work now; previews are locked until native activation (mainnet H279,370).",
        annotations(
            title = "Native read",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn native_read(
        &self,
        Parameters(args): Parameters<NativeReadArgs>,
    ) -> Result<String, String> {
        self.native_read_impl(args).await
    }

    #[tool(
        description = "Submit a previewed native token/NFT/trade operation or dismiss a pending record. Requires mode wallet and native activation; the preview plan_id binds the request.",
        annotations(
            title = "Native write",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn native_write(
        &self,
        Parameters(args): Parameters<NativeWriteArgs>,
    ) -> Result<String, String> {
        self.native_write_impl(args).await
    }

    #[tool(
        description = "Peer count and every aggregate P2P/quorum metric the node exposes. The node has no per-peer IP/port/latency RPC; the report states that explicitly.",
        annotations(
            title = "Peer report",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn peer_report(&self) -> Result<String, String> {
        self.peer_report_impl().await
    }

    #[tool(
        description = "Mining info and peers; getBlockTemplate/submitBlock require mode admin and an external-miner node.",
        annotations(
            title = "Mining",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn mining(&self, Parameters(args): Parameters<MiningArgs>) -> Result<String, String> {
        self.mining_impl(args).await
    }

    #[tool(
        description = "Gracefully stop the node. Requires mode admin and the exact confirmation bound to the current height. Use sparingly; it is the owner's mainnet node.",
        annotations(
            title = "Node control",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn node_control(
        &self,
        Parameters(args): Parameters<NodeControlArgs>,
    ) -> Result<String, String> {
        self.node_control_impl(args).await
    }

    #[tool(
        description = "Read-only passthrough for any of the 79 RPC methods, with the same parameter, mode and activation checks. Write/control methods are refused; use their dedicated tools.",
        annotations(
            title = "Raw RPC (read-only)",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn rpc_call(&self, Parameters(args): Parameters<RpcCallArgs>) -> Result<String, String> {
        self.rpc_call_impl(args).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Parano1dServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "parano1d-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
            format!(
                "Parano1d node MCP at {}. Current mode: {}. Read-only is the default. \
                 wallet_send needs wallet mode, a plan_id from wallet_plan_send bound to the active source \
                 address, the exact confirm string and PARANOID_MCP_MAX_SEND_MICRONOID. The node's own wallet signs; never ask for or use a \
                 private key. Native v2.1 (tokens, NFTs, market, DEX) is gated by the node's activation height \
                 (mainnet H279370); native_write unlocks automatically. rpc_call is read-only and refuses \
                 write/control methods.",
                self.rpc.url(),
                self.cfg.mode.as_str()
            ),
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RpcCallArgs {
    #[schemars(description = "RPC suffix without the paranoid_ prefix, e.g. getBlockDetails")]
    pub method: String,
    #[schemars(description = "Positional JSON-RPC parameters. Omit for no parameters.")]
    pub params: Option<Vec<Value>>,
}

impl Parano1dServer {
    pub(crate) async fn node_overview_impl(&self) -> Result<String, String> {
        let capabilities = self.caps.get(true).await;
        let calls: Vec<(&str, Vec<Value>)> = vec![
            ("getNodeStatus", vec![]),
            ("getMiningInfo", vec![]),
            ("getPeerCount", vec![]),
            ("getMempoolStats", vec![]),
            ("walletStatus", vec![]),
        ];
        let labels = ["node", "mining", "peers", "mempool", "wallet"];
        debug_assert_eq!(labels.len(), calls.len());
        let results = self.rpc.batch_read(&calls, Duration::from_secs(20)).await;
        let mut output = Map::new();
        for (label, result) in labels.iter().zip(results) {
            match result {
                Ok(value) => {
                    output.insert((*label).to_string(), value);
                }
                Err(error) => {
                    output.insert((*label).to_string(), json!({"error": error.to_json()}));
                }
            }
        }
        output.insert(
            "chain".into(),
            capabilities
                .chain_info_raw
                .clone()
                .unwrap_or_else(|| json!({"error": "chain probe failed; see capabilities.errors"})),
        );
        output.insert(
            "contracts".into(),
            capabilities.contract_protocol_raw.clone().unwrap_or_else(
                || json!({"error": "contract protocol probe failed; see capabilities.errors"}),
            ),
        );
        output.insert(
            "native".into(),
            capabilities.native_protocol_raw.clone().unwrap_or_else(
                || json!({"error": "native protocol probe failed; see capabilities.errors"}),
            ),
        );
        output.insert(
            "capabilities".into(),
            serde_json::to_value(&capabilities).unwrap_or(Value::Null),
        );
        output.insert("mode".into(), json!(self.cfg.mode.as_str()));
        output.insert("rpc_url".into(), json!(self.rpc.url()));
        output.insert(
            "rpc_methods".into(),
            json!({
                "total": manifest::METHODS.len(),
                "v21_additions": manifest::METHODS.iter().filter(|method| method.since_2_1).count()
            }),
        );
        self.capped(Value::Object(output))
    }

    pub(crate) async fn rpc_call_impl(&self, args: RpcCallArgs) -> Result<String, String> {
        let spec = manifest::find(&args.method).ok_or_else(|| {
            format!(
                "unknown RPC method {:?}; not in the 79-method manifest",
                args.method
            )
        })?;
        if spec.access != Access::Read {
            return Err(format!(
                "{} is a write/control method ({}); use its dedicated MCP tool instead of rpc_call",
                args.method, spec.summary
            ));
        }
        let params = args.params.unwrap_or_default();
        let value = self.call_read(&args.method, params).await?;
        self.capped(value)
    }

    pub(crate) async fn check_method(
        &self,
        method: &str,
        param_count: usize,
    ) -> Result<&'static MethodSpec, String> {
        let spec = manifest::find(method).ok_or_else(|| {
            format!("unknown RPC method {method:?}; not in the 79-method manifest")
        })?;
        if !self.cfg.mode.allows(spec.access) {
            return Err(format!(
                "{method} requires mode {}; current mode is {}. Change PARANOID_MCP_MODE and restart the MCP server.",
                access_label(spec.access),
                self.cfg.mode.as_str()
            ));
        }
        let capabilities = self.caps.get(false).await;
        if let Some(error) = capabilities.gate_error(spec.gate) {
            return Err(error);
        }
        if param_count < spec.min_params || param_count > spec.max_params {
            return Err(format!(
                "{method} expects {}..={} positional params, got {param_count}",
                spec.min_params, spec.max_params
            ));
        }
        Ok(spec)
    }

    pub(crate) async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value, String> {
        let spec = self.check_method(method, params.len()).await?;
        let retry = spec.access == Access::Read;
        self.rpc
            .call(
                method,
                params,
                timeout_for(method, self.cfg.request_timeout),
                retry,
            )
            .await
            .map_err(|error| {
                if spec.access == Access::Read {
                    rpc_error_string(error)
                } else {
                    // A write whose response was lost may still have been
                    // accepted by the node. Say so instead of reporting a
                    // failure, and never retry it.
                    if error.outcome_unknown() {
                        tracing::warn!(method, "write outcome unknown");
                    }
                    write_error_string(error)
                }
            })
    }

    pub(crate) async fn call_read(
        &self,
        method: &str,
        params: Vec<Value>,
    ) -> Result<Value, String> {
        let spec = self.check_method(method, params.len()).await?;
        if spec.access != Access::Read {
            return Err(format!("{method} is not a read method"));
        }
        self.rpc
            .call(
                method,
                params,
                timeout_for(method, self.cfg.request_timeout),
                true,
            )
            .await
            .map_err(rpc_error_string)
    }

    /// The limit applies to the text actually returned (pretty JSON), and
    /// the value is serialized once on the normal path.
    pub(crate) fn capped(&self, value: Value) -> Result<String, String> {
        let pretty = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        if pretty.len() > self.cfg.max_result_bytes {
            let compact = serde_json::to_string(&value).map_err(|e| e.to_string())?;
            let preview: String = compact.chars().take(300).collect();
            let payload = json!({
                "truncated": true,
                "bytes": pretty.len(),
                "limit_bytes": self.cfg.max_result_bytes,
                "preview": preview,
                "note": "Result exceeded PARANOID_MCP_MAX_RESULT_BYTES. Narrow the query or raise the limit."
            });
            return serde_json::to_string_pretty(&payload).map_err(|e| e.to_string());
        }
        Ok(pretty)
    }

    pub(crate) fn require_wallet_cap(&self) -> Result<u64, String> {
        self.cfg.max_send_micronoid.ok_or_else(|| {
            "wallet writes are disabled: set PARANOID_MCP_MAX_SEND_MICRONOID to a positive value and restart the MCP server".into()
        })
    }

    pub(crate) fn enforce_fee_cap(&self, fee_micronoid: u64) -> Result<(), String> {
        if let Some(max_fee) = self.cfg.max_fee_micronoid
            && fee_micronoid > max_fee
        {
            return Err(format!(
                "fee {fee_micronoid} μNOID exceeds PARANOID_MCP_MAX_FEE_MICRONOID {max_fee}"
            ));
        }
        Ok(())
    }

    /// One place decides whether a spend is inside the per-call cap, so the
    /// five spending paths cannot drift apart. `total` is always amount plus
    /// fee (native: debit plus fee).
    pub(crate) fn enforce_send_cap(
        &self,
        what: &str,
        total: u64,
        max_send: u64,
    ) -> Result<(), String> {
        if total > max_send {
            return Err(format!(
                "{what} {total} μNOID exceeds PARANOID_MCP_MAX_SEND_MICRONOID {max_send}"
            ));
        }
        Ok(())
    }
}

fn access_label(access: Access) -> &'static str {
    match access {
        Access::Read => "read",
        Access::Wallet => "wallet",
        Access::Admin => "admin",
    }
}

fn timeout_for(method: &str, default: Duration) -> Duration {
    match method {
        "getBlockTemplate" => Duration::from_secs(180),
        "walletSend" | "walletConsolidate" | "walletSubmitNative" | "walletFundObject"
        | "walletCallObject" | "submitTxIntent" => Duration::from_secs(120),
        _ => default,
    }
}

pub(crate) fn rpc_error_string(error: RpcError) -> String {
    match &error {
        RpcError::JsonRpc {
            code,
            message,
            data,
        } => {
            // A local node is trusted, but its error message and data are
            // still bounded so a malformed response cannot flood the model's
            // context.
            let mut text = format!("JSON-RPC {code}: {}", bounded(message));
            if let Some(data) = data {
                text.push_str("; data=");
                text.push_str(&bounded(&data.to_string()));
            }
            text
        }
        _ => error.to_string(),
    }
}

/// At most 2,000 characters of node-supplied error text.
fn bounded(text: &str) -> String {
    text.chars().take(2000).collect()
}

/// Error text for a write. A rejected request (a JSON-RPC error) is a definite
/// failure; a lost or malformed response is not, because the node may already
/// have applied the write. Those are reported as *unknown*, with a hint to
/// check the chain, and are never retried.
pub(crate) fn write_error_string(error: RpcError) -> String {
    if error.outcome_unknown() {
        format!(
            "outcome unknown: the request may have reached the node but its response was lost or unreadable ({error}). Do not resend blindly; check getTx, wallet history, the mempool or the active address first."
        )
    } else {
        rpc_error_string(error)
    }
}

pub(crate) fn require_string(args: Option<String>, name: &str) -> Result<String, String> {
    args.ok_or_else(|| format!("{name} is required for this action"))
}

pub(crate) fn require_u64(args: Option<u64>, name: &str) -> Result<u64, String> {
    args.ok_or_else(|| format!("{name} is required for this action"))
}

pub(crate) fn require_value(args: Option<Value>, name: &str) -> Result<Value, String> {
    args.ok_or_else(|| format!("{name} is required for this action"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_error_data_is_bounded() {
        let error = RpcError::JsonRpc {
            code: -32000,
            message: "node error".into(),
            data: Some(json!("a".repeat(10_000))),
        };
        let text = rpc_error_string(error);
        assert!(text.len() < 2_200, "error text was not bounded");
        assert!(text.contains("data="), "error data was dropped: {text}");
    }

    #[test]
    fn rpc_error_message_is_bounded_too() {
        // The message comes from the node as well; a hostile one must not be
        // able to put megabytes of text into one tool error.
        let error = RpcError::JsonRpc {
            code: -32000,
            message: "m".repeat(100_000),
            data: Some(json!("d".repeat(100_000))),
        };
        let text = rpc_error_string(error.clone());
        assert!(text.len() < 4_200, "error text was {} bytes", text.len());
        assert!(text.starts_with("JSON-RPC -32000: mmm"), "{}", &text[..40]);
        assert!(text.contains("; data=\"ddd"), "error data was dropped");
        assert!(write_error_string(error).len() < 4_200);
    }

    #[test]
    fn the_result_limit_applies_to_the_text_returned() {
        let config = Config {
            rpc_url: crate::config::DEFAULT_RPC_URL.to_string(),
            mode: crate::config::Mode::Read,
            max_send_micronoid: None,
            max_fee_micronoid: None,
            max_result_bytes: 1024,
            request_timeout: Duration::from_secs(1),
            allow_raw_intent: false,
        };
        let server = Parano1dServer::new(config).expect("server");
        // Deep nesting: small as compact JSON, large once pretty-printed.
        let mut value = json!(1);
        for _ in 0..40 {
            value = json!([value]);
        }
        let compact = serde_json::to_string(&value).expect("compact");
        let pretty = serde_json::to_string_pretty(&value).expect("pretty");
        assert!(compact.len() <= 1024 && pretty.len() > 1024, "test premise");
        let text = server.capped(value).expect("capped");
        assert!(text.len() <= 1024, "returned {} bytes", text.len());
        let summary: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(summary["truncated"], true);
        assert_eq!(summary["bytes"], pretty.len());
        // Under the limit, the pretty text is returned unchanged.
        assert_eq!(
            server.capped(json!({"a": 1})).expect("small"),
            "{\n  \"a\": 1\n}"
        );
    }

    #[test]
    fn send_cap_boundary_is_exact() {
        let config = Config {
            rpc_url: crate::config::DEFAULT_RPC_URL.to_string(),
            mode: crate::config::Mode::Wallet,
            max_send_micronoid: Some(500),
            max_fee_micronoid: None,
            max_result_bytes: crate::config::DEFAULT_MAX_RESULT_BYTES,
            request_timeout: Duration::from_secs(1),
            allow_raw_intent: false,
        };
        let server = Parano1dServer::new(config).expect("server");
        // Exactly at the cap is allowed; one μNOID over is refused. The cap
        // covers amount + fee on every spending path.
        assert!(server.enforce_send_cap("amount + fee", 500, 500).is_ok());
        let error = server
            .enforce_send_cap("amount + fee", 501, 500)
            .unwrap_err();
        assert!(error.contains("PARANOID_MCP_MAX_SEND_MICRONOID"), "{error}");
        assert!(server.enforce_send_cap("amount + fee", 0, 0).is_ok());
        assert!(server.enforce_send_cap("amount + fee", 1, 0).is_err());
    }

    #[test]
    fn no_node_errors_name_the_url_and_where_to_get_a_node() {
        let error = RpcError::Unreachable {
            url: "http://127.0.0.1:9601".into(),
            detail: "Connection refused (os error 61)".into(),
        };
        let expected = "No Parano1d node answered at http://127.0.0.1:9601. Install and start one: https://parano1d.org (Downloads), then retry.";
        let read = rpc_error_string(error.clone());
        assert!(read.starts_with(expected), "{read}");
        assert!(
            read.ends_with("(Connection refused (os error 61))"),
            "{read}"
        );
        // A failed connect sent nothing, so a write is a definite failure.
        let write = write_error_string(error);
        assert!(write.starts_with(expected), "{write}");
        assert!(!write.contains("outcome unknown"), "{write}");
    }

    #[test]
    fn lost_write_responses_are_reported_as_unknown() {
        for error in [
            RpcError::Transport("timeout".into()),
            RpcError::Invalid("response id mismatch".into()),
            RpcError::Http {
                status: 500,
                body: "boom".into(),
            },
        ] {
            assert!(
                write_error_string(error).contains("outcome unknown"),
                "a lost response must not read as a failure"
            );
        }
        // A node rejection is a definite failure, not an unknown outcome.
        let rejected = RpcError::JsonRpc {
            code: -32000,
            message: "insufficient funds".into(),
            data: None,
        };
        let text = write_error_string(rejected);
        assert!(!text.contains("outcome unknown"), "{text}");
        assert!(text.contains("insufficient funds"), "{text}");
        let bad_request = RpcError::Http {
            status: 400,
            body: "bad request".into(),
        };
        assert!(!write_error_string(bad_request).contains("outcome unknown"));
        // A request refused for size never left the process, so it is not an
        // unknown outcome either.
        let too_large = RpcError::RequestTooLarge(1234);
        let text = write_error_string(too_large);
        assert!(!text.contains("outcome unknown"), "{text}");
        assert!(text.contains("1234"), "{text}");
    }
}
