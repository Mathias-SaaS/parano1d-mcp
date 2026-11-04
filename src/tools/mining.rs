// SPDX-License-Identifier: Apache-2.0
//! Mining and node-control tools.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::{Parano1dServer, require_string};
use crate::safety::confirm_stop;
use crate::util::field_u64;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MiningAction {
    Info,
    Peers,
    BlockTemplate,
    SubmitBlock,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiningArgs {
    #[schemars(description = "Which mining action to run")]
    pub action: MiningAction,
    #[schemars(description = "Payout address for block_template (empty uses the node payout)")]
    pub miner_address: Option<String>,
    #[schemars(description = "Template id from block_template")]
    pub template_id: Option<String>,
    #[schemars(description = "32 lowercase hex characters = 16 little-endian nonce bytes")]
    pub nonce_hex: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn mining_impl(&self, args: MiningArgs) -> Result<String, String> {
        let value = match args.action {
            MiningAction::Info => self.call_read("getMiningInfo", vec![]).await?,
            MiningAction::Peers => self.call_read("getPeerCount", vec![]).await?,
            MiningAction::BlockTemplate => {
                let address = args.miner_address.unwrap_or_default();
                self.call("getBlockTemplate", vec![json!(address)]).await?
            }
            MiningAction::SubmitBlock => {
                let template_id = require_string(args.template_id, "template_id")?;
                let nonce_hex = require_string(args.nonce_hex, "nonce_hex")?;
                // The node accepts only canonical lowercase hex.
                if nonce_hex.len() != 32
                    || !nonce_hex
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err("nonce_hex must be exactly 32 lowercase hex characters".into());
                }
                self.call("submitBlock", vec![json!(template_id), json!(nonce_hex)])
                    .await?
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeControlAction {
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeControlArgs {
    #[schemars(description = "Which control action to run")]
    pub action: NodeControlAction,
    #[schemars(description = "Exact confirmation: STOP AT HEIGHT <current height>")]
    pub confirm: String,
}

impl Parano1dServer {
    pub(crate) async fn node_control_impl(&self, args: NodeControlArgs) -> Result<String, String> {
        match args.action {
            NodeControlAction::Stop => {
                self.check_method("stop", 0).await?;
                let chain = self.call_read("getChainInfo", vec![]).await?;
                let height = field_u64(&chain, "height")?;
                let expected = confirm_stop(height);
                if args.confirm != expected {
                    return Err(format!(
                        "confirmation mismatch; pass confirm = {expected:?} (the current height is included so a stale command cannot stop the node)"
                    ));
                }
                let result = self.call("stop", vec![]).await?;
                self.capped(json!({
                    "result": result,
                    "note": "The node is shutting down gracefully. The MCP server process is independent and will keep running, but all further RPC calls fail until the node returns."
                }))
            }
        }
    }
}
