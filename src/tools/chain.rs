// SPDX-License-Identifier: Apache-2.0
//! Chain, block, state, mempool and fee tools.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Parano1dServer, require_string, require_u64};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChainAction {
    BlockCount,
    ChainInfo,
    NodeStatus,
    BlockHash,
    BlockHeader,
    BlockHeaderByHash,
    HeaderByHeight,
    HeaderByHash,
    HistoryStepTerminal,
    Tx,
    Slot,
    SlotsByOwner,
    ActiveSlotCount,
    StateInfo,
    StateMap,
    RecentTransactions,
    Block,
    BlockDetails,
    VerifyReceipt,
    SlotHints,
    SlotHintsSalted,
    EpochAnchor,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChainQueryArgs {
    #[schemars(description = "Which chain query to run")]
    pub action: ChainAction,
    #[schemars(description = "Block height, where the action needs one")]
    pub height: Option<u64>,
    #[schemars(description = "Block hash or transaction id, where the action needs one")]
    pub hash: Option<String>,
    #[schemars(description = "State slot index")]
    pub slot_index: Option<u32>,
    #[schemars(description = "Owner address (o1…)")]
    pub address: Option<String>,
    #[schemars(description = "Page number, starting at 1")]
    pub page: Option<u32>,
    #[schemars(description = "Page size (recent transactions: 1..=32)")]
    pub page_size: Option<u32>,
    #[schemars(description = "Number of slot hints (1..=256)")]
    pub count: Option<u32>,
    #[schemars(description = "Hex salt for salted slot hints")]
    pub salt_hex: Option<String>,
    #[schemars(
        description = "Include the full raw block hex. Default false returns only its size."
    )]
    pub include_raw: Option<bool>,
    #[schemars(description = "Hex-encoded payment receipt for verify_receipt")]
    pub receipt_hex: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn chain_query_impl(&self, args: ChainQueryArgs) -> Result<String, String> {
        let value = match args.action {
            ChainAction::BlockCount => self.call_read("blockCount", vec![]).await?,
            ChainAction::ChainInfo => self.call_read("getChainInfo", vec![]).await?,
            ChainAction::NodeStatus => self.call_read("getNodeStatus", vec![]).await?,
            ChainAction::BlockHash => {
                let height = require_u64(args.height, "height")?;
                self.call_read("getBlockHash", vec![json!(height)]).await?
            }
            ChainAction::BlockHeader => {
                let height = require_u64(args.height, "height")?;
                self.call_read("getBlockHeader", vec![json!(height)])
                    .await?
            }
            ChainAction::BlockHeaderByHash => {
                let hash = require_string(args.hash, "hash")?;
                self.call_read("getBlockHeaderByHash", vec![json!(hash)])
                    .await?
            }
            ChainAction::HeaderByHeight => {
                let height = require_u64(args.height, "height")?;
                self.call_read("getHeaderByHeight", vec![json!(height)])
                    .await?
            }
            ChainAction::HeaderByHash => {
                let hash = require_string(args.hash, "hash")?;
                self.call_read("getHeaderByHash", vec![json!(hash)]).await?
            }
            ChainAction::HistoryStepTerminal => {
                self.call_read("getHistoryStepTerminal", vec![]).await?
            }
            ChainAction::Tx => {
                let hash = require_string(args.hash, "hash")?;
                self.call_read("getTx", vec![json!(hash)]).await?
            }
            ChainAction::Slot => {
                let slot_index = args
                    .slot_index
                    .ok_or_else(|| "slot_index is required for this action".to_string())?;
                self.call_read("getSlot", vec![json!(slot_index)]).await?
            }
            ChainAction::SlotsByOwner => {
                let address = require_string(args.address, "address")?;
                self.call_read("getSlotsByOwner", vec![json!(address)])
                    .await?
            }
            ChainAction::ActiveSlotCount => self.call_read("getActiveSlotCount", vec![]).await?,
            ChainAction::StateInfo => self.call_read("getStateInfo", vec![]).await?,
            ChainAction::StateMap => self.call_read("getStateMap", vec![]).await?,
            ChainAction::RecentTransactions => {
                let page = args.page.unwrap_or(1).max(1);
                let page_size = args.page_size.unwrap_or(10).clamp(1, 32);
                let address = args.address.map(Value::String).unwrap_or(Value::Null);
                self.call_read(
                    "getRecentTransactions",
                    vec![json!(page), json!(page_size), address],
                )
                .await?
            }
            ChainAction::Block => {
                let height = require_u64(args.height, "height")?;
                let value = self.call_read("getBlock", vec![json!(height)]).await?;
                if args.include_raw.unwrap_or(false) {
                    value
                } else {
                    match value {
                        Value::Null => json!({"height": height, "retained": false}),
                        Value::String(hex) => json!({
                            "height": height,
                            "retained": true,
                            "bytes": hex.len() / 2,
                            "raw_omitted": true,
                            "hint": "call again with include_raw=true to receive the full block hex"
                        }),
                        other => other,
                    }
                }
            }
            ChainAction::BlockDetails => {
                let height = require_u64(args.height, "height")?;
                self.call_read("getBlockDetails", vec![json!(height)])
                    .await?
            }
            ChainAction::VerifyReceipt => {
                let receipt = require_string(args.receipt_hex, "receipt_hex")?;
                self.call_read("verifyReceipt", vec![json!(receipt)])
                    .await?
            }
            ChainAction::SlotHints => {
                let count = args.count.unwrap_or(16).clamp(1, 256);
                self.call_read("getSlotHints", vec![json!(count)]).await?
            }
            ChainAction::SlotHintsSalted => {
                let count = args.count.unwrap_or(16).clamp(1, 256);
                let salt = require_string(args.salt_hex, "salt_hex")?;
                self.call_read("getSlotHintsSalted", vec![json!(count), json!(salt)])
                    .await?
            }
            ChainAction::EpochAnchor => self.call_read("getEpochAnchor", vec![]).await?,
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MempoolAction {
    Info,
    Size,
    Stats,
    Entry,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MempoolQueryArgs {
    #[schemars(description = "Which mempool query to run")]
    pub action: MempoolAction,
    #[schemars(description = "Transaction id for the entry action")]
    pub txid: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn mempool_query_impl(
        &self,
        args: MempoolQueryArgs,
    ) -> Result<String, String> {
        let value = match args.action {
            MempoolAction::Info => self.call_read("getMempoolInfo", vec![]).await?,
            MempoolAction::Size => self.call_read("getMempoolSize", vec![]).await?,
            MempoolAction::Stats => self.call_read("getMempoolStats", vec![]).await?,
            MempoolAction::Entry => {
                let txid = require_string(args.txid, "txid")?;
                self.call_read("getMempoolEntry", vec![json!(txid)]).await?
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeeEstimateArgs {
    #[schemars(description = "Live input count (1..=1020). Omit for the one-input estimator.")]
    pub n_inputs: Option<u32>,
    #[schemars(description = "Output count (1..=256)")]
    pub n_outputs: u32,
}

impl Parano1dServer {
    pub(crate) async fn fee_estimate_impl(&self, args: FeeEstimateArgs) -> Result<String, String> {
        if !(1..=256).contains(&args.n_outputs) {
            return Err("n_outputs must be 1..=256".into());
        }
        let value = match args.n_inputs {
            Some(n_inputs) => {
                if !(1..=1020).contains(&n_inputs) {
                    return Err("n_inputs must be 1..=1020".into());
                }
                self.call_read(
                    "estimateFeeDetailed",
                    vec![json!(n_inputs), json!(args.n_outputs)],
                )
                .await?
            }
            None => {
                self.call_read("estimateFee", vec![json!(args.n_outputs)])
                    .await?
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddressValidateArgs {
    #[schemars(description = "Address to validate (o1…)")]
    pub address: String,
}

impl Parano1dServer {
    pub(crate) async fn address_validate_impl(
        &self,
        args: AddressValidateArgs,
    ) -> Result<String, String> {
        let value = self
            .call_read("validateAddress", vec![json!(args.address)])
            .await?;
        self.capped(value)
    }
}
