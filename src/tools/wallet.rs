// SPDX-License-Identifier: Apache-2.0
//! Wallet reads, guarded sends and local wallet maintenance.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Parano1dServer, require_string, require_value};
use crate::safety::{PLAN_TTL, confirm_send, confirm_set_active};
use crate::util::{field_u64, total_spend};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalletReadAction {
    Status,
    Balance,
    Address,
    ListAddresses,
    ActiveAddress,
    Utxos,
    UtxoSnapshot,
    History,
    Receipts,
    QueryReceipts,
    MinedBlocks,
    ExportReceipt,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletReadArgs {
    #[schemars(description = "Which wallet query to run")]
    pub action: WalletReadAction,
    #[schemars(description = "Key index for the address action")]
    pub index: Option<u32>,
    #[schemars(description = "Page number, starting at 1")]
    pub page: Option<u32>,
    #[schemars(description = "Page size (1..=50)")]
    pub page_size: Option<u32>,
    #[schemars(description = "Receipt family: all, noid, tokens, trades or nfts")]
    pub filter: Option<String>,
    #[schemars(description = "ReceiptQuery object for query_receipts (v2.1)")]
    pub query: Option<Value>,
    #[schemars(description = "Transaction id for export_receipt")]
    pub txid: Option<String>,
    #[schemars(description = "Opaque revision from a previous utxo_snapshot")]
    pub known_revision: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn wallet_read_impl(&self, args: WalletReadArgs) -> Result<String, String> {
        let value = match args.action {
            WalletReadAction::Status => self.call_read("walletStatus", vec![]).await?,
            WalletReadAction::Balance => self.call_read("walletGetBalance", vec![]).await?,
            WalletReadAction::Address => {
                let index = args
                    .index
                    .ok_or_else(|| "index is required for the address action".to_string())?;
                self.call_read("walletGetAddress", vec![json!(index)])
                    .await?
            }
            WalletReadAction::ListAddresses => {
                self.call_read("walletListAddresses", vec![]).await?
            }
            WalletReadAction::ActiveAddress => {
                self.call_read("walletActiveAddress", vec![]).await?
            }
            WalletReadAction::Utxos => self.call_read("walletListUtxos", vec![]).await?,
            WalletReadAction::UtxoSnapshot => match args.known_revision {
                Some(revision) => {
                    self.call_read("walletUtxoSnapshot", vec![json!(revision)])
                        .await?
                }
                None => self.call_read("walletUtxoSnapshot", vec![]).await?,
            },
            WalletReadAction::History => self.call_read("walletHistory", vec![]).await?,
            WalletReadAction::Receipts => {
                let page = args.page.unwrap_or(1).max(1);
                let page_size = args.page_size.unwrap_or(20).clamp(1, 50);
                let mut params = vec![json!(page), json!(page_size)];
                if let Some(filter) = args.filter {
                    params.push(json!(filter));
                }
                self.call_read("walletReceipts", params).await?
            }
            WalletReadAction::QueryReceipts => {
                let query = require_value(args.query, "query")?;
                let page = args.page.unwrap_or(1).max(1);
                let page_size = args.page_size.unwrap_or(20).clamp(1, 50);
                self.call_read(
                    "walletQueryReceipts",
                    vec![json!(page), json!(page_size), query],
                )
                .await?
            }
            WalletReadAction::MinedBlocks => {
                let page = args.page.unwrap_or(1).max(1);
                let page_size = args.page_size.unwrap_or(20).clamp(1, 50);
                self.call_read("walletMinedBlocks", vec![json!(page), json!(page_size)])
                    .await?
            }
            WalletReadAction::ExportReceipt => {
                let txid = require_string(args.txid, "txid")?;
                self.call_read("walletExportReceipt", vec![json!(txid)])
                    .await?
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletPlanSendArgs {
    #[schemars(description = "Recipient address (o1…)")]
    pub to: String,
    #[schemars(description = "Amount in μNOID (1 NOID = 1,000,000 μNOID)")]
    pub amount_micronoid: u64,
    #[schemars(description = "Exact fee in μNOID; 0 or omitted asks the node to choose")]
    pub fee_micronoid: Option<u64>,
    #[schemars(
        description = "Source key index; it must already be active. Omit to use the current active address."
    )]
    pub from_index: Option<u32>,
}

impl Parano1dServer {
    /// Read the active key index and address from the node. Every send plan is
    /// bound to this source; the node has no source parameter on walletSend, so
    /// a changed active address must force a new quote.
    async fn active_wallet(&self) -> Result<(u32, String), String> {
        let value = self.call_read("walletActiveAddress", vec![]).await?;
        let index = u32::try_from(field_u64(&value, "key_index")?)
            .map_err(|_| "walletActiveAddress key_index exceeds u32".to_string())?;
        let address = value
            .get("address")
            .and_then(Value::as_str)
            .ok_or_else(|| "walletActiveAddress returned no address".to_string())?
            .to_string();
        Ok((index, address))
    }

    /// `mainnet` or `testnet`, from the node itself. A node that does not
    /// report the flag is refused rather than guessed at.
    async fn node_network(&self) -> Result<String, String> {
        let value = self.call_read("getNodeStatus", vec![]).await?;
        match value.get("testnet").and_then(Value::as_bool) {
            Some(true) => Ok("testnet".to_string()),
            Some(false) => Ok("mainnet".to_string()),
            None => Err(
                "getNodeStatus did not report \"testnet\"; refusing to plan or send without knowing the network"
                    .to_string(),
            ),
        }
    }

    pub(crate) async fn wallet_plan_send_impl(
        &self,
        args: WalletPlanSendArgs,
    ) -> Result<String, String> {
        let _wallet_ops = self.wallet_ops.lock().await;
        if args.amount_micronoid == 0 {
            return Err("amount_micronoid must be greater than zero".into());
        }
        if let Some(max_send) = self.cfg.max_send_micronoid
            && args.amount_micronoid > max_send
        {
            return Err(format!(
                "amount {} μNOID exceeds PARANOID_MCP_MAX_SEND_MICRONOID {max_send}",
                args.amount_micronoid
            ));
        }
        let fee = args.fee_micronoid.unwrap_or(0);
        if fee > 0 {
            self.enforce_fee_cap(fee)?;
        }
        let (active_index, active_address) = self.active_wallet().await?;
        let network = self.node_network().await?;
        if let Some(from) = args.from_index
            && from != active_index
        {
            return Err(format!(
                "source index {from} is not active (active index {active_index}, {active_address}); call wallet_maintenance set_active_address with confirm {:?} first, then call wallet_plan_send again",
                confirm_set_active(from)
            ));
        }
        let plan = self
            .call_read(
                "walletPlanSend",
                vec![json!(args.to), json!(args.amount_micronoid), json!(fee)],
            )
            .await?;
        let (after_index, after_address) = self.active_wallet().await?;
        if after_index != active_index || after_address != active_address {
            return Err(format!(
                "active address changed while quoting (index {active_index} -> {after_index}); call wallet_plan_send again"
            ));
        }
        // The node-chosen fee is not the requested fee. Apply the optional fee
        // cap to the quote as well, not only to an explicit fee argument.
        let quoted_fee = field_u64(&plan, "fee_micronoid")?;
        self.enforce_fee_cap(quoted_fee)?;
        if let Some(max_send) = self.cfg.max_send_micronoid {
            let total = total_spend(args.amount_micronoid, quoted_fee)?;
            self.enforce_send_cap("amount + quoted fee", total, max_send)?;
        }
        let plan_id = self.plans.put_send(
            active_index,
            &active_address,
            &network,
            &args.to,
            args.amount_micronoid,
            fee,
            plan.clone(),
        );
        self.capped(json!({
            "plan_id": plan_id,
            "expires_in_seconds": PLAN_TTL.as_secs(),
            "from_index": active_index,
            "from_address": active_address,
            "network": network,
            "requested_fee_micronoid": fee,
            "quoted_fee_micronoid": quoted_fee,
            "plan": plan,
            "note": "Review the plan. To execute, call wallet_send with this plan_id, the same destination, amount and fee argument, and the same source index (the plan is bound to it); include the exact confirm string. wallet_send re-quotes and refuses if the quote or active address changed."
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletSendArgs {
    #[schemars(description = "Recipient address (o1…)")]
    pub to: String,
    #[schemars(description = "Amount in μNOID")]
    pub amount_micronoid: u64,
    #[schemars(
        description = "Fee argument in μNOID; pass the same value used in wallet_plan_send (0 or omitted if the node chose the fee)"
    )]
    pub fee_micronoid: Option<u64>,
    #[schemars(
        description = "Source key index; it must already be active and must match the plan. Omit to use the current active address."
    )]
    pub from_index: Option<u32>,
    #[schemars(description = "plan_id returned by wallet_plan_send")]
    pub plan_id: String,
    #[schemars(
        description = "Exact confirmation string: SEND <amount_micronoid> MICRONOID TO <address>"
    )]
    pub confirm: String,
}

impl Parano1dServer {
    pub(crate) async fn wallet_send_impl(&self, args: WalletSendArgs) -> Result<String, String> {
        // Check the mode before the cap so read mode reports the mode error.
        self.check_method("walletSend", 3).await?;
        let max_send = self.require_wallet_cap()?;
        if args.amount_micronoid == 0 {
            return Err("amount_micronoid must be greater than zero".into());
        }
        if args.amount_micronoid > max_send {
            return Err(format!(
                "amount {} μNOID exceeds PARANOID_MCP_MAX_SEND_MICRONOID {max_send}",
                args.amount_micronoid
            ));
        }
        let fee = args.fee_micronoid.unwrap_or(0);
        if fee > 0 {
            self.enforce_fee_cap(fee)?;
        }
        let expected = confirm_send(&args.to, args.amount_micronoid);
        if args.confirm != expected {
            return Err(format!(
                "confirmation mismatch; pass confirm = {expected:?} exactly"
            ));
        }

        let _wallet_ops = self.wallet_ops.lock().await;
        let (active_index, active_address) = self.active_wallet().await?;
        let network = self.node_network().await?;
        let cached = self.plans.take_send(
            &args.plan_id,
            &args.to,
            args.amount_micronoid,
            fee,
            active_index,
            &active_address,
            &network,
            args.from_index,
        )?;

        let fresh = self
            .call_read(
                "walletPlanSend",
                vec![json!(args.to), json!(args.amount_micronoid), json!(fee)],
            )
            .await?;
        let (fresh_index, fresh_address) = self.active_wallet().await?;
        if fresh_index != active_index || fresh_address != active_address {
            return Err(format!(
                "active address changed while preparing the send (index {active_index} -> {fresh_index}); call wallet_plan_send again"
            ));
        }
        let fresh_fee = field_u64(&fresh, "fee_micronoid")?;
        let total = total_spend(args.amount_micronoid, fresh_fee)?;
        self.enforce_send_cap("amount + fee", total, max_send)?;
        let cached_print = send_fingerprint(&cached.plan)?;
        let fresh_print = send_fingerprint(&fresh)?;
        if cached_print != fresh_print {
            return Err(format!(
                "quote changed since wallet_plan_send: cached {:?}, now {:?}; call wallet_plan_send again",
                cached_print, fresh_print
            ));
        }

        // Bind the send to the quoted fee. A zero request means "choose a
        // fee"; passing the quoted value back prevents the node from choosing
        // a different, larger fee between the re-quote and the write. The
        // resulting fee must satisfy the optional fee cap too.
        let send_fee = send_fee_within_cap(fee, fresh_fee, self.cfg.max_fee_micronoid)?;
        let result = self
            .call(
                "walletSend",
                vec![
                    json!(args.to),
                    json!(args.amount_micronoid),
                    json!(send_fee),
                ],
            )
            .await?;
        self.capped(json!({
            "result": result,
            "source": {
                "key_index": active_index,
                "address": active_address
            },
            "network": network,
            "note": "Submitted to the mempool, not necessarily included in a block. Check getTx or wallet history for confirmation."
        }))
    }
}

fn send_fingerprint(plan: &Value) -> Result<(u64, u64, u64, u64, u64, u64), String> {
    Ok((
        field_u64(plan, "amount_micronoid")?,
        field_u64(plan, "fee_micronoid")?,
        field_u64(plan, "total_spend_micronoid")?,
        field_u64(plan, "input_count")?,
        field_u64(plan, "output_count")?,
        field_u64(plan, "change_micronoid")?,
    ))
}

/// Resolve the fee that will actually be sent and apply the optional fee cap.
/// A zero request means the node chose the quoted fee, so the quote must be
/// checked too; checking only the literal zero would bypass the cap.
fn send_fee_within_cap(requested: u64, quoted: u64, max_fee: Option<u64>) -> Result<u64, String> {
    let send_fee = if requested == 0 { quoted } else { requested };
    if let Some(max_fee) = max_fee
        && send_fee > max_fee
    {
        return Err(format!(
            "fee {send_fee} μNOID exceeds PARANOID_MCP_MAX_FEE_MICRONOID {max_fee}"
        ));
    }
    Ok(send_fee)
}

#[cfg(test)]
mod tests {
    use super::send_fee_within_cap;

    #[test]
    fn zero_requested_fee_is_still_capped() {
        assert!(send_fee_within_cap(0, 9000, Some(1)).is_err());
        assert_eq!(send_fee_within_cap(0, 9000, Some(9000)).unwrap(), 9000);
        assert_eq!(send_fee_within_cap(5000, 9000, Some(5000)).unwrap(), 5000);
        assert_eq!(send_fee_within_cap(0, 0, None).unwrap(), 0);
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TxSubmitArgs {
    #[schemars(description = "Hex-encoded signed PagedSpendIntent / native intent")]
    pub intent_hex: String,
    #[schemars(description = "Exact confirmation: SUBMIT SIGNED INTENT")]
    pub confirm: String,
}

impl Parano1dServer {
    pub(crate) async fn tx_submit_impl(&self, args: TxSubmitArgs) -> Result<String, String> {
        self.check_method("submitTxIntent", 1).await?;
        if !self.cfg.allow_raw_intent {
            return Err(
                "raw signed intent submission is disabled; set PARANOID_MCP_ALLOW_RAW_INTENT=1 and restart the MCP server"
                    .into(),
            );
        }
        if args.confirm != "SUBMIT SIGNED INTENT" {
            return Err(
                "confirmation mismatch; pass confirm = \"SUBMIT SIGNED INTENT\" exactly".into(),
            );
        }
        let result = self
            .call("submitTxIntent", vec![json!(args.intent_hex)])
            .await?;
        self.capped(json!({
            "result": result,
            "note": "Raw signed intent submitted to the mempool. The send-plan cap cannot inspect an already-signed intent; wallet mode plus PARANOID_MCP_ALLOW_RAW_INTENT=1 is the gate. Check getTx for confirmation."
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalletMaintenanceAction {
    Scan,
    DiscoverAddresses,
    NextAddress,
    SetActiveAddress,
    PlanConsolidation,
    Consolidate,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletMaintenanceArgs {
    #[schemars(description = "Which local wallet action to run")]
    pub action: WalletMaintenanceAction,
    #[schemars(description = "How many addresses to discover (1..=20)")]
    pub max_additional: Option<u32>,
    #[schemars(description = "Key index for set_active_address")]
    pub index: Option<u32>,
    #[schemars(
        description = "Exact confirmation for set_active_address: SET ACTIVE ADDRESS <index>"
    )]
    pub confirm: Option<String>,
    #[schemars(description = "plan_id returned by plan_consolidation, for consolidate")]
    pub plan_id: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn wallet_maintenance_impl(
        &self,
        args: WalletMaintenanceArgs,
    ) -> Result<String, String> {
        let value = match args.action {
            WalletMaintenanceAction::Scan => {
                let _wallet_ops = self.wallet_ops.lock().await;
                self.call("walletScan", vec![]).await?
            }
            WalletMaintenanceAction::DiscoverAddresses => {
                let max_additional = args.max_additional.unwrap_or(5);
                if !(1..=20).contains(&max_additional) {
                    return Err("max_additional must be 1..=20".into());
                }
                let _wallet_ops = self.wallet_ops.lock().await;
                self.call("walletDiscoverAddresses", vec![json!(max_additional)])
                    .await?
            }
            WalletMaintenanceAction::NextAddress => {
                let _wallet_ops = self.wallet_ops.lock().await;
                self.call("walletNextAddress", vec![]).await?
            }
            WalletMaintenanceAction::SetActiveAddress => {
                let index = args
                    .index
                    .ok_or_else(|| "index is required for set_active_address".to_string())?;
                self.check_method("walletSetActiveAddress", 1).await?;
                let _wallet_ops = self.wallet_ops.lock().await;
                let expected = confirm_set_active(index);
                if args.confirm.as_deref() != Some(expected.as_str()) {
                    return Err(format!(
                        "confirmation mismatch; pass confirm = {expected:?} exactly. This changes the active signing address and mining payout."
                    ));
                }
                self.call("walletSetActiveAddress", vec![json!(index)])
                    .await?
            }
            WalletMaintenanceAction::PlanConsolidation => {
                let _wallet_ops = self.wallet_ops.lock().await;
                let plan = self.call_read("walletPlanConsolidation", vec![]).await?;
                let plan_id = self.plans.put_consolidation(&plan)?;
                return self.capped(json!({
                    "plan_id": plan_id,
                    "expires_in_seconds": PLAN_TTL.as_secs(),
                    "plan": plan,
                    "note": "To execute, call this tool again with action consolidate and this plan_id. The quote is re-checked by the node."
                }));
            }
            WalletMaintenanceAction::Consolidate => {
                let plan_id = args
                    .plan_id
                    .ok_or_else(|| "plan_id is required for consolidate".to_string())?;
                self.check_method("walletConsolidate", 3).await?;
                let _wallet_ops = self.wallet_ops.lock().await;
                let max_send = self.require_wallet_cap()?;
                let plan = self.plans.take_consolidation(&plan_id)?;
                let total = total_spend(0, plan.expected_fee_micronoid)?;
                self.enforce_send_cap("consolidation fee", total, max_send)?;
                self.enforce_fee_cap(plan.expected_fee_micronoid)?;
                self.call(
                    "walletConsolidate",
                    vec![
                        json!(plan.selected_input_slots),
                        json!(plan.expected_fee_micronoid),
                        json!(plan.expected_output_value_micronoid),
                    ],
                )
                .await?
            }
        };
        self.capped(value)
    }
}
