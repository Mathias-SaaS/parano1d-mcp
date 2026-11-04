// SPDX-License-Identifier: Apache-2.0
//! v2.1 native tokens, NFTs, marketplace and DEX tools.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Parano1dServer, require_string, require_value};
use crate::safety::{NativePlan, PLAN_TTL};
use crate::util::{total_spend, value_u64};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeReadAction {
    Protocol,
    Catalog,
    Market,
    Holdings,
    WalletHoldings,
    Pending,
    Preview,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeReadArgs {
    #[schemars(description = "Which native read to run")]
    pub action: NativeReadAction,
    #[schemars(description = "Catalog kind: markets, collections or nft_listings")]
    pub kind: Option<String>,
    #[schemars(description = "Opaque catalogue/holdings cursor")]
    pub cursor: Option<Value>,
    #[schemars(description = "Row limit")]
    pub limit: Option<u32>,
    #[schemars(description = "64-character asset id")]
    pub asset_id: Option<String>,
    #[schemars(description = "Owner address (o1…)")]
    pub owner: Option<String>,
    #[schemars(description = "NativeWalletRequest JSON for preview")]
    pub request: Option<Value>,
}

impl Parano1dServer {
    pub(crate) async fn native_read_impl(&self, args: NativeReadArgs) -> Result<String, String> {
        let value = match args.action {
            NativeReadAction::Protocol => self.call_read("getNativeProtocol", vec![]).await?,
            NativeReadAction::Catalog => {
                let kind = require_string(args.kind, "kind")?;
                if !matches!(kind.as_str(), "markets" | "collections" | "nft_listings") {
                    return Err("kind must be markets, collections or nft_listings".into());
                }
                let cursor = args.cursor.unwrap_or(Value::Null);
                let limit = args.limit.unwrap_or(20).clamp(1, 100);
                self.call_read("getNativeCatalog", vec![json!(kind), cursor, json!(limit)])
                    .await?
            }
            NativeReadAction::Market => {
                let asset_id = require_string(args.asset_id, "asset_id")?;
                self.call_read("getNativeMarket", vec![json!(asset_id)])
                    .await?
            }
            NativeReadAction::Holdings => {
                let owner = require_string(args.owner, "owner")?;
                let cursor = args.cursor.unwrap_or(Value::Null);
                let limit = args.limit.unwrap_or(20).clamp(1, 100);
                self.call_read(
                    "getNativeHoldings",
                    vec![json!(owner), cursor, json!(limit)],
                )
                .await?
            }
            NativeReadAction::WalletHoldings => {
                let cursor = args.cursor.unwrap_or(Value::Null);
                let limit = args.limit.unwrap_or(20).clamp(1, 100);
                self.call_read("walletGetNativeHoldings", vec![cursor, json!(limit)])
                    .await?
            }
            NativeReadAction::Pending => self.call_read("walletNativePending", vec![]).await?,
            NativeReadAction::Preview => {
                let request = require_value(args.request, "request")?;
                let preview = self
                    .call_read("walletPreviewNative", vec![request.clone()])
                    .await?;
                let plan_id = self.plans.put_native(request, &preview)?;
                return self.capped(json!({
                    "plan_id": plan_id,
                    "expires_in_seconds": PLAN_TTL.as_secs(),
                    "preview": preview,
                    "note": "Review the preview. To submit, call native_write with action submit, this plan_id and the identical request. The node also binds its own reviewed plan_id."
                }));
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeWriteAction {
    Submit,
    DismissPending,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeWriteArgs {
    #[schemars(description = "Which native write to run")]
    pub action: NativeWriteAction,
    #[schemars(description = "The exact NativeWalletRequest JSON that was previewed")]
    pub request: Option<Value>,
    #[schemars(description = "plan_id returned by native_read preview")]
    pub plan_id: Option<String>,
    #[schemars(description = "Native pending transaction id for dismiss_pending")]
    pub txid: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn native_write_impl(&self, args: NativeWriteArgs) -> Result<String, String> {
        let value = match args.action {
            NativeWriteAction::Submit => {
                self.check_method("walletSubmitNative", 2).await?;
                let _wallet_ops = self.wallet_ops.lock().await;
                let max_send = self.require_wallet_cap()?;
                let request = require_value(args.request, "request")?;
                let plan_id = require_string(args.plan_id, "plan_id")?;
                let plan = self.plans.take_native(&plan_id, &request)?;
                let (what, total, fee) = native_spend_bound(&request, &plan)?;
                self.enforce_send_cap(what, total, max_send)?;
                self.enforce_fee_cap(fee)?;
                self.call(
                    "walletSubmitNative",
                    vec![request, json!(plan.node_plan_id)],
                )
                .await?
            }
            NativeWriteAction::DismissPending => {
                let txid = require_string(args.txid, "txid")?;
                self.call("walletDismissNativePending", vec![json!(txid)])
                    .await?
            }
        };
        self.capped(value)
    }
}

/// The most NOID a previewed native operation is authorised to cost, and the
/// highest fee it may pay. Every operation's debit and fee are exact except a
/// trade: it signs only `gross_input` and `fee_limit`, and the block it lands
/// in decides the fill. A trade is therefore capped at whichever is larger, its
/// quote or what it signs (a buy: `gross_input + fee_limit`; a sell spends
/// tokens, so `fee_limit`).
fn native_spend_bound(
    request: &Value,
    plan: &NativePlan,
) -> Result<(&'static str, u64, u64), String> {
    let quoted = total_spend(plan.noid_debit_micronoid, plan.fee_micronoid)?;
    if request.get("action").and_then(Value::as_str) != Some("trade") {
        return Ok(("native NOID debit + fee", quoted, plan.fee_micronoid));
    }
    let fee_limit = plan.fee_limit_micronoid.ok_or_else(|| {
        "trade preview has no fee_limit_micronoid, so its worst-case cost is unknown; refusing"
            .to_string()
    })?;
    let buy = request
        .get("buy")
        .and_then(Value::as_bool)
        .ok_or_else(|| "trade request has no boolean buy".to_string())?;
    let signed = if buy {
        let gross_input = request
            .get("gross_input")
            .and_then(value_u64)
            .ok_or_else(|| "trade request gross_input is not a u64".to_string())?;
        total_spend(gross_input, fee_limit)?
    } else {
        fee_limit
    };
    Ok((
        "native trade worst case (signed gross_input + fee_limit, or the quote if larger)",
        quoted.max(signed),
        plan.fee_micronoid.max(fee_limit),
    ))
}
