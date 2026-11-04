// SPDX-License-Identifier: Apache-2.0
//! Proof-native contract (ABI 3) tools.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Parano1dServer, require_string, require_value};
use crate::util::{optional_field_u64, total_spend, value_u64};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContractReadAction {
    Protocol,
    PreviewCall,
    Status,
    Instances,
    GetOpening,
    ListStates,
    ListReceipts,
    ExportReceipt,
    VerifyReceipt,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractReadArgs {
    #[schemars(description = "Which contract read to run")]
    pub action: ContractReadAction,
    #[schemars(description = "Public opening hex")]
    pub opening_hex: Option<String>,
    #[schemars(description = "Slot index for status")]
    pub slot_index: Option<u32>,
    #[schemars(description = "Inclusive starting slot for instances")]
    pub from_slot: Option<u32>,
    #[schemars(description = "Row limit (instances 1..=256; states/receipts 1..=64)")]
    pub limit: Option<u32>,
    #[schemars(description = "Object address (o1…)")]
    pub address: Option<String>,
    #[schemars(description = "Exclusive state cursor")]
    pub after_root: Option<String>,
    #[schemars(description = "Exclusive activity cursor")]
    pub after_cursor: Option<String>,
    #[schemars(description = "Transaction id for export_receipt")]
    pub txid: Option<String>,
    #[schemars(description = "Hex-encoded contract receipt")]
    pub receipt_hex: Option<String>,
    #[schemars(description = "ObjectCallRequest JSON for preview_call")]
    pub request: Option<Value>,
}

impl Parano1dServer {
    pub(crate) async fn contract_read_impl(
        &self,
        args: ContractReadArgs,
    ) -> Result<String, String> {
        let value = match args.action {
            ContractReadAction::Protocol => self.call_read("getContractProtocol", vec![]).await?,
            ContractReadAction::PreviewCall => {
                let request = require_value(args.request, "request")?;
                self.call_read("previewObjectCall", vec![request]).await?
            }
            ContractReadAction::Status => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let slot = args
                    .slot_index
                    .ok_or_else(|| "slot_index is required for status".to_string())?;
                self.call_read("getObjectStatus", vec![json!(opening), json!(slot)])
                    .await?
            }
            ContractReadAction::Instances => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let from_slot = args.from_slot.unwrap_or(0);
                let limit = args.limit.unwrap_or(50).clamp(1, 256);
                self.call_read(
                    "getObjectInstances",
                    vec![json!(opening), json!(from_slot), json!(limit)],
                )
                .await?
            }
            ContractReadAction::GetOpening => {
                let address = require_string(args.address, "address")?;
                self.call_read("walletGetObjectOpening", vec![json!(address)])
                    .await?
            }
            ContractReadAction::ListStates => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let cursor = args.after_root.map(Value::String).unwrap_or(Value::Null);
                let limit = args.limit.unwrap_or(32).clamp(1, 64);
                self.call_read(
                    "walletListObjectStates",
                    vec![json!(opening), cursor, json!(limit)],
                )
                .await?
            }
            ContractReadAction::ListReceipts => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let cursor = args.after_cursor.map(Value::String).unwrap_or(Value::Null);
                let limit = args.limit.unwrap_or(32).clamp(1, 64);
                self.call_read(
                    "walletListObjectReceipts",
                    vec![json!(opening), cursor, json!(limit)],
                )
                .await?
            }
            ContractReadAction::ExportReceipt => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let txid = require_string(args.txid, "txid")?;
                self.call_read("exportObjectReceipt", vec![json!(opening), json!(txid)])
                    .await?
            }
            ContractReadAction::VerifyReceipt => {
                let receipt = require_string(args.receipt_hex, "receipt_hex")?;
                self.call_read("verifyObjectReceipt", vec![json!(receipt)])
                    .await?
            }
        };
        self.capped(value)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContractWriteAction {
    Create,
    Fund,
    Call,
    Watch,
    ImportReceipt,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractWriteArgs {
    #[schemars(description = "Which contract write to run")]
    pub action: ContractWriteAction,
    #[schemars(description = "Public opening hex")]
    pub opening_hex: Option<String>,
    #[schemars(description = "ObjectDefinition JSON for create")]
    pub definition: Option<Value>,
    #[schemars(description = "ObjectCallRequest JSON for call")]
    pub request: Option<Value>,
    #[schemars(description = "Funding amount in μNOID")]
    pub amount_micronoid: Option<u64>,
    #[schemars(description = "Fee in μNOID")]
    pub fee_micronoid: Option<u64>,
    #[schemars(description = "Expected sender address for fund")]
    pub expected_sender: Option<String>,
    #[schemars(description = "Hex-encoded contract receipt for import_receipt")]
    pub receipt_hex: Option<String>,
    #[schemars(description = "Expected opening hex for import_receipt")]
    pub expected_opening_hex: Option<String>,
}

impl Parano1dServer {
    pub(crate) async fn contract_write_impl(
        &self,
        args: ContractWriteArgs,
    ) -> Result<String, String> {
        let value = match args.action {
            ContractWriteAction::Create => {
                let definition = require_value(args.definition, "definition")?;
                self.call("createObject", vec![definition]).await?
            }
            ContractWriteAction::Fund => {
                self.check_method("walletFundObject", 3).await?;
                let _wallet_ops = self.wallet_ops.lock().await;
                let max_send = self.require_wallet_cap()?;
                let opening = require_string(args.opening_hex, "opening_hex")?;
                let amount = args
                    .amount_micronoid
                    .ok_or_else(|| "amount_micronoid is required for fund".to_string())?;
                let fee = args
                    .fee_micronoid
                    .ok_or_else(|| "fee_micronoid is required for contract funding".to_string())?;
                let fee = explicit_fee(fee, "contract funding")?;
                let total = total_spend(amount, fee)?;
                self.enforce_send_cap("amount + fee", total, max_send)?;
                self.enforce_fee_cap(fee)?;
                let mut params = vec![json!(opening), json!(amount), json!(fee)];
                if let Some(sender) = args.expected_sender {
                    params.push(json!(sender));
                }
                self.call("walletFundObject", params).await?
            }
            ContractWriteAction::Call => {
                self.check_method("walletCallObject", 1).await?;
                let _wallet_ops = self.wallet_ops.lock().await;
                let max_send = self.require_wallet_cap()?;
                let request = require_value(args.request, "request")?;
                let (fee, payout) = contract_call_spend(&request)?;
                let fee = explicit_fee(fee, "contract call")?;
                let total = total_spend(payout, fee)?;
                self.enforce_send_cap("contract payout + fee", total, max_send)?;
                self.enforce_fee_cap(fee)?;
                // The request shows only an explicit payout. A terminal call
                // has none, yet releases the whole balance to the contract's
                // committed recipient, so cap what the node previews.
                let preview = self
                    .call_read("previewObjectCall", vec![request.clone()])
                    .await?;
                let (fee, released) =
                    contract_call_spend(&preview).map_err(|e| format!("previewObjectCall: {e}"))?;
                let total = total_spend(released, fee)?;
                self.enforce_send_cap("previewed contract release + fee", total, max_send)?;
                self.enforce_fee_cap(fee)?;
                self.call("walletCallObject", vec![request]).await?
            }
            ContractWriteAction::Watch => {
                let opening = require_string(args.opening_hex, "opening_hex")?;
                self.call("walletWatchObject", vec![json!(opening)]).await?
            }
            ContractWriteAction::ImportReceipt => {
                let receipt = require_string(args.receipt_hex, "receipt_hex")?;
                let mut params = vec![json!(receipt)];
                if let Some(opening) = args.expected_opening_hex {
                    params.push(json!(opening));
                }
                self.call("walletImportObjectReceipt", params).await?
            }
        };
        self.capped(value)
    }
}

/// A zero contract fee means "use the current minimum", whose value the MCP
/// cannot verify. Capped contract writes therefore need an explicit fee from
/// the preview or estimator.
fn explicit_fee(fee_micronoid: u64, operation: &str) -> Result<u64, String> {
    if fee_micronoid == 0 {
        Err(format!(
            "{operation} requires an explicit non-zero fee; obtain it from the preview or estimator and pass it explicitly"
        ))
    } else {
        Ok(fee_micronoid)
    }
}

/// Extract the spend-relevant values from an `ObjectCallRequest` (or the
/// `ObjectCallPreview` of one, which has the same two fields) for the local
/// cap checks. Present-but-malformed fields fail closed; missing optional
/// fields are zero.
fn contract_call_spend(request: &Value) -> Result<(u64, u64), String> {
    let fee = optional_field_u64(request, "fee_micronoid")?.unwrap_or(0);
    let payout = match request.get("payout") {
        None | Some(Value::Null) => 0,
        Some(payout) => {
            let amount = payout.get("amount_micronoid").ok_or_else(|| {
                "contract payout is present but missing amount_micronoid".to_string()
            })?;
            value_u64(amount)
                .ok_or_else(|| "contract payout amount_micronoid is not a u64".to_string())?
        }
    };
    Ok((fee, payout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn call_spend_parses_numbers_and_strings() {
        assert_eq!(
            contract_call_spend(&json!({
                "fee_micronoid": "5000",
                "payout": {"address": "o1x", "amount_micronoid": 100}
            }))
            .unwrap(),
            (5000, 100)
        );
    }

    #[test]
    fn call_spend_defaults_missing_optional_fields_to_zero() {
        assert_eq!(contract_call_spend(&json!({})).unwrap(), (0, 0));
        assert_eq!(
            contract_call_spend(&json!({"payout": null})).unwrap(),
            (0, 0)
        );
    }

    #[test]
    fn call_spend_fails_closed_on_malformed_fields() {
        assert!(contract_call_spend(&json!({"fee_micronoid": "abc"})).is_err());
        assert!(contract_call_spend(&json!({"payout": {"address": "o1x"}})).is_err());
        assert!(
            contract_call_spend(&json!({
                "payout": {"address": "o1x", "amount_micronoid": "abc"}
            }))
            .is_err()
        );
    }

    #[test]
    fn contract_fees_must_be_explicit() {
        assert!(explicit_fee(0, "contract call").is_err());
        assert_eq!(explicit_fee(1, "contract call").unwrap(), 1);
    }
}
