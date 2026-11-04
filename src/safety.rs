// SPDX-License-Identifier: Apache-2.0
//! Short-lived plan binding for spending operations.
//!
//! The node's send API has no plan id of its own. This store gives every quote a
//! one-time id, requires the caller to restate the exact destination and amount,
//! and re-quotes immediately before the write.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use uuid::Uuid;

use crate::util::{field_u64, optional_field_u64};

pub const PLAN_TTL: Duration = Duration::from_secs(120);
/// Live plans kept per kind. Inside the TTL the stores are otherwise
/// unbounded, so a client looping on quotes could grow memory.
pub const MAX_PLANS: usize = 64;

#[derive(Debug, Clone)]
pub struct SendPlan {
    pub from_index: u32,
    pub from_address: String,
    /// `mainnet` or `testnet`, from `getNodeStatus.testnet` at plan time.
    pub network: String,
    pub to: String,
    pub amount_micronoid: u64,
    pub fee_micronoid: u64,
    pub plan: Value,
    created: Instant,
}

#[derive(Debug, Clone)]
pub struct ConsolidationPlan {
    pub selected_input_slots: Vec<u32>,
    pub expected_fee_micronoid: u64,
    pub expected_output_value_micronoid: u64,
    created: Instant,
}

#[derive(Debug, Clone)]
pub struct NativePlan {
    /// The node's own reviewed plan id, passed to walletSubmitNative.
    pub node_plan_id: String,
    pub request: Value,
    pub fee_micronoid: u64,
    /// The highest fee a trade may pay; the node reports it for trades only.
    pub fee_limit_micronoid: Option<u64>,
    pub noid_debit_micronoid: u64,
    created: Instant,
}

#[derive(Default)]
pub struct PlanStore {
    sends: Mutex<HashMap<String, SendPlan>>,
    consolidations: Mutex<HashMap<String, ConsolidationPlan>>,
    natives: Mutex<HashMap<String, NativePlan>>,
}

impl PlanStore {
    pub fn new() -> Self {
        Self::default()
    }

    // Every bound field of the plan is an explicit argument, so a caller
    // cannot forget one.
    #[allow(clippy::too_many_arguments)]
    pub fn put_send(
        &self,
        from_index: u32,
        from_address: &str,
        network: &str,
        to: &str,
        amount_micronoid: u64,
        fee_micronoid: u64,
        plan: Value,
    ) -> String {
        self.put_send_at(
            from_index,
            from_address,
            network,
            to,
            amount_micronoid,
            fee_micronoid,
            plan,
            Instant::now(),
        )
    }

    // Same arguments as `put_send`, plus the injected clock.
    #[allow(clippy::too_many_arguments)]
    fn put_send_at(
        &self,
        from_index: u32,
        from_address: &str,
        network: &str,
        to: &str,
        amount_micronoid: u64,
        fee_micronoid: u64,
        plan: Value,
        now: Instant,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let entry = SendPlan {
            from_index,
            from_address: from_address.to_string(),
            network: network.to_string(),
            to: to.to_string(),
            amount_micronoid,
            fee_micronoid,
            plan,
            created: now,
        };
        let mut sends = self
            .sends
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        admit(&mut sends, now);
        sends.insert(id.clone(), entry);
        id
    }

    // The caller must restate every bound field it is redeeming.
    #[allow(clippy::too_many_arguments)]
    pub fn take_send(
        &self,
        id: &str,
        to: &str,
        amount_micronoid: u64,
        fee_micronoid: u64,
        active_index: u32,
        active_address: &str,
        network: &str,
        requested_from: Option<u32>,
    ) -> Result<SendPlan, String> {
        self.take_send_at(
            id,
            to,
            amount_micronoid,
            fee_micronoid,
            active_index,
            active_address,
            network,
            requested_from,
            Instant::now(),
        )
    }

    // Same arguments as `take_send`, plus the injected clock.
    #[allow(clippy::too_many_arguments)]
    fn take_send_at(
        &self,
        id: &str,
        to: &str,
        amount_micronoid: u64,
        fee_micronoid: u64,
        active_index: u32,
        active_address: &str,
        network: &str,
        requested_from: Option<u32>,
        now: Instant,
    ) -> Result<SendPlan, String> {
        let plan = self
            .sends
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
            .ok_or_else(|| {
                "unknown or already used plan_id; call wallet_plan_send first".to_string()
            })?;
        if now.duration_since(plan.created) > PLAN_TTL {
            return Err("plan expired; call wallet_plan_send again".into());
        }
        if plan.to != to
            || plan.amount_micronoid != amount_micronoid
            || plan.fee_micronoid != fee_micronoid
        {
            return Err("plan_id does not match the destination, amount and fee; call wallet_plan_send again".into());
        }
        if let Some(from) = requested_from
            && from != plan.from_index
        {
            return Err(format!(
                "plan_id belongs to source index {}; requested source index {from}; call wallet_plan_send again",
                plan.from_index
            ));
        }
        if plan.network != network {
            return Err(format!(
                "network changed since wallet_plan_send: plan is for {}, the node now reports {network}; call wallet_plan_send again",
                plan.network
            ));
        }
        if plan.from_index != active_index || plan.from_address != active_address {
            return Err(format!(
                "active address changed since wallet_plan_send: plan source is index {} ({}), current active is index {} ({active_address}); call wallet_plan_send again",
                plan.from_index, plan.from_address, active_index
            ));
        }
        Ok(plan)
    }

    pub fn put_consolidation(&self, plan: &Value) -> Result<String, String> {
        self.put_consolidation_at(plan, Instant::now())
    }

    fn put_consolidation_at(&self, plan: &Value, now: Instant) -> Result<String, String> {
        let selected_input_slots = plan
            .get("selected_input_slots")
            .and_then(Value::as_array)
            .ok_or_else(|| "plan has no selected_input_slots".to_string())?
            .iter()
            .map(|slot| {
                slot.as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| "invalid selected_input_slots entry".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expected_fee_micronoid = field_u64(plan, "fee_micronoid")?;
        let expected_output_value_micronoid = field_u64(plan, "output_value_micronoid")?;
        let id = Uuid::new_v4().to_string();
        let entry = ConsolidationPlan {
            selected_input_slots,
            expected_fee_micronoid,
            expected_output_value_micronoid,
            created: now,
        };
        let mut plans = self
            .consolidations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        admit(&mut plans, now);
        plans.insert(id.clone(), entry);
        Ok(id)
    }

    pub fn take_consolidation(&self, id: &str) -> Result<ConsolidationPlan, String> {
        self.take_consolidation_at(id, Instant::now())
    }

    fn take_consolidation_at(&self, id: &str, now: Instant) -> Result<ConsolidationPlan, String> {
        let plan = self
            .consolidations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
            .ok_or_else(|| "unknown or already used consolidation plan_id".to_string())?;
        if now.duration_since(plan.created) > PLAN_TTL {
            return Err("consolidation plan expired; call wallet_plan_consolidation again".into());
        }
        Ok(plan)
    }

    pub fn put_native(&self, request: Value, preview: &Value) -> Result<String, String> {
        self.put_native_at(request, preview, Instant::now())
    }

    fn put_native_at(
        &self,
        request: Value,
        preview: &Value,
        now: Instant,
    ) -> Result<String, String> {
        let node_plan_id = preview
            .get("plan_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "preview has no plan_id".to_string())?
            .to_string();
        let fee_micronoid = field_u64(preview, "fee_micronoid")?;
        let fee_limit_micronoid = optional_field_u64(preview, "fee_limit_micronoid")?;
        let noid_debit_micronoid = field_u64(preview, "noid_debit_micronoid")?;
        let id = Uuid::new_v4().to_string();
        let entry = NativePlan {
            node_plan_id,
            request,
            fee_micronoid,
            fee_limit_micronoid,
            noid_debit_micronoid,
            created: now,
        };
        let mut plans = self
            .natives
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        admit(&mut plans, now);
        plans.insert(id.clone(), entry);
        Ok(id)
    }

    pub fn take_native(&self, id: &str, request: &Value) -> Result<NativePlan, String> {
        self.take_native_at(id, request, Instant::now())
    }

    fn take_native_at(
        &self,
        id: &str,
        request: &Value,
        now: Instant,
    ) -> Result<NativePlan, String> {
        let plan = self
            .natives
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
            .ok_or_else(|| {
                "unknown or already used native plan_id; call native_read preview first".to_string()
            })?;
        if now.duration_since(plan.created) > PLAN_TTL {
            return Err("native preview expired; preview again".into());
        }
        if &plan.request != request {
            return Err("request does not match the previewed request; preview again".into());
        }
        Ok(plan)
    }
}

fn prune<T: HasCreated>(map: &mut HashMap<String, T>, now: Instant) {
    map.retain(|_, entry| now.duration_since(entry.created()) <= PLAN_TTL);
}

/// Make room for one more plan: drop expired ones, then the oldest until
/// fewer than `MAX_PLANS` remain.
fn admit<T: HasCreated>(map: &mut HashMap<String, T>, now: Instant) {
    prune(map, now);
    while map.len() >= MAX_PLANS {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, entry)| entry.created())
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
}

trait HasCreated {
    fn created(&self) -> Instant;
}

impl HasCreated for SendPlan {
    fn created(&self) -> Instant {
        self.created
    }
}

impl HasCreated for ConsolidationPlan {
    fn created(&self) -> Instant {
        self.created
    }
}

impl HasCreated for NativePlan {
    fn created(&self) -> Instant {
        self.created
    }
}

pub fn confirm_send(to: &str, amount_micronoid: u64) -> String {
    format!("SEND {amount_micronoid} MICRONOID TO {to}")
}

pub fn confirm_stop(height: u64) -> String {
    format!("STOP AT HEIGHT {height}")
}

pub fn confirm_set_active(index: u32) -> String {
    format!("SET ACTIVE ADDRESS {index}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn send_plan_is_one_time_and_bound() {
        let store = PlanStore::new();
        let id = store.put_send(
            1,
            "o1active",
            "mainnet",
            "o1abc",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        // A mismatched use consumes the plan; the caller must re-quote.
        assert!(
            store
                .take_send(&id, "o1other", 100, 5, 1, "o1active", "mainnet", None)
                .is_err()
        );
        assert!(
            store
                .take_send(&id, "o1abc", 100, 5, 1, "o1active", "mainnet", None)
                .is_err()
        );

        let id = store.put_send(
            1,
            "o1active",
            "mainnet",
            "o1abc",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        assert!(
            store
                .take_send(&id, "o1abc", 100, 5, 1, "o1active", "mainnet", None)
                .is_ok()
        );
        assert!(
            store
                .take_send(&id, "o1abc", 100, 5, 1, "o1active", "mainnet", None)
                .is_err()
        );
    }

    #[test]
    fn send_plan_binds_source() {
        let store = PlanStore::new();
        let id = store.put_send(
            3,
            "o1src",
            "mainnet",
            "o1to",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 2, "o1src", "mainnet", Some(3))
                .is_err()
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 3, "o1src", "mainnet", Some(3))
                .is_err()
        );

        let id = store.put_send(
            3,
            "o1src",
            "mainnet",
            "o1to",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 3, "o1src", "mainnet", Some(2))
                .is_err()
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 3, "o1src", "mainnet", Some(3))
                .is_err()
        );

        let id = store.put_send(
            3,
            "o1src",
            "mainnet",
            "o1to",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 3, "o1other", "mainnet", None)
                .is_err()
        );
        assert!(
            store
                .take_send(&id, "o1to", 100, 5, 3, "o1src", "mainnet", None)
                .is_err()
        );

        let id = store.put_send(
            3,
            "o1src",
            "mainnet",
            "o1to",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        let plan = store
            .take_send(&id, "o1to", 100, 5, 3, "o1src", "mainnet", Some(3))
            .unwrap();
        assert_eq!(plan.from_index, 3);
        assert_eq!(plan.from_address, "o1src");
    }

    #[test]
    fn send_plan_expires() {
        let store = PlanStore::new();
        let id = store.put_send_at(
            1,
            "o1active",
            "mainnet",
            "o1abc",
            100,
            5,
            json!({}),
            Instant::now(),
        );
        let later = Instant::now() + PLAN_TTL + Duration::from_secs(1);
        assert!(
            store
                .take_send_at(&id, "o1abc", 100, 5, 1, "o1active", "mainnet", None, later)
                .is_err()
        );
        assert!(
            store
                .take_send_at(&id, "o1abc", 100, 5, 1, "o1active", "mainnet", None, later)
                .is_err()
        );
    }

    #[test]
    fn consolidation_plan_parses_quote() {
        let store = PlanStore::new();
        let id = store
            .put_consolidation(&json!({
                "selected_input_slots": [1, 2, 3],
                "fee_micronoid": 9000,
                "output_value_micronoid": 100000
            }))
            .unwrap();
        let plan = store.take_consolidation(&id).unwrap();
        assert_eq!(plan.selected_input_slots, vec![1, 2, 3]);
        assert_eq!(plan.expected_fee_micronoid, 9000);
    }

    #[test]
    fn native_plan_binds_request() {
        let store = PlanStore::new();
        let request = json!({"action": "trade", "asset_id": "aa", "buy": true, "gross_input": "1"});
        let id = store
            .put_native(
                request.clone(),
                &json!({"plan_id": "node-plan", "fee_micronoid": "5000", "noid_debit_micronoid": "1000"}),
            )
            .unwrap();
        assert!(
            store
                .take_native(&id, &json!({"action": "burn_token"}))
                .is_err()
        );
        let id = store
            .put_native(
                request.clone(),
                &json!({"plan_id": "node-plan", "fee_micronoid": "5000", "noid_debit_micronoid": "1000"}),
            )
            .unwrap();
        let plan = store.take_native(&id, &request).unwrap();
        assert_eq!(plan.node_plan_id, "node-plan");
        assert_eq!(plan.noid_debit_micronoid, 1000);
    }

    #[test]
    fn send_plan_binds_the_network() {
        let store = PlanStore::new();
        let id = store.put_send(
            1,
            "o1active",
            "mainnet",
            "o1abc",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        // The plan is consumed by the mismatched attempt; a mainnet plan must
        // never authorise a send on a testnet node.
        let error = store
            .take_send(&id, "o1abc", 100, 5, 1, "o1active", "testnet", None)
            .unwrap_err();
        assert!(error.contains("network changed"), "{error}");

        let id = store.put_send(
            1,
            "o1active",
            "testnet",
            "o1abc",
            100,
            5,
            json!({"fee_micronoid": 5}),
        );
        let plan = store
            .take_send(&id, "o1abc", 100, 5, 1, "o1active", "testnet", None)
            .unwrap();
        assert_eq!(plan.network, "testnet");
    }

    #[test]
    fn send_plan_binds_every_field_independently() {
        let store = PlanStore::new();
        // Same destination and amount, different fee.
        let id = store.put_send(1, "o1active", "mainnet", "o1abc", 100, 5, json!({}));
        assert!(
            store
                .take_send(&id, "o1abc", 100, 6, 1, "o1active", "mainnet", None)
                .is_err(),
            "a plan quoted with fee 5 must not authorise fee 6"
        );
        // Same destination and fee, different amount.
        let id = store.put_send(1, "o1active", "mainnet", "o1abc", 100, 5, json!({}));
        assert!(
            store
                .take_send(&id, "o1abc", 101, 5, 1, "o1active", "mainnet", None)
                .is_err(),
            "a plan quoted for 100 must not authorise 101"
        );
        // Different destination only.
        let id = store.put_send(1, "o1active", "mainnet", "o1abc", 100, 5, json!({}));
        assert!(
            store
                .take_send(&id, "o1xyz", 100, 5, 1, "o1active", "mainnet", None)
                .is_err(),
            "a plan must not be redirected to another destination"
        );
    }

    #[test]
    fn plan_ttl_accepts_exactly_the_ttl_and_refuses_one_nanosecond_more() {
        let store = PlanStore::new();
        let now = Instant::now();
        let id = store.put_send_at(1, "o1active", "mainnet", "o1abc", 100, 5, json!({}), now);
        assert!(
            store
                .take_send_at(
                    &id,
                    "o1abc",
                    100,
                    5,
                    1,
                    "o1active",
                    "mainnet",
                    None,
                    now + PLAN_TTL
                )
                .is_ok()
        );
        let id = store.put_send_at(1, "o1active", "mainnet", "o1abc", 100, 5, json!({}), now);
        assert!(
            store
                .take_send_at(
                    &id,
                    "o1abc",
                    100,
                    5,
                    1,
                    "o1active",
                    "mainnet",
                    None,
                    now + PLAN_TTL + Duration::from_nanos(1)
                )
                .is_err()
        );

        let plan = json!({
            "selected_input_slots": [1],
            "fee_micronoid": 9000,
            "output_value_micronoid": 100000
        });
        let id = store.put_consolidation_at(&plan, now).unwrap();
        assert!(store.take_consolidation_at(&id, now + PLAN_TTL).is_ok());
        let id = store.put_consolidation_at(&plan, now).unwrap();
        assert!(
            store
                .take_consolidation_at(&id, now + PLAN_TTL + Duration::from_nanos(1))
                .is_err()
        );

        let request = json!({"action": "burn_token", "asset_id": "aa", "units": "1"});
        let preview = json!({"plan_id": "p", "fee_micronoid": "1", "noid_debit_micronoid": "0"});
        let id = store.put_native_at(request.clone(), &preview, now).unwrap();
        assert!(store.take_native_at(&id, &request, now + PLAN_TTL).is_ok());
        let id = store.put_native_at(request.clone(), &preview, now).unwrap();
        assert!(
            store
                .take_native_at(&id, &request, now + PLAN_TTL + Duration::from_nanos(1))
                .is_err()
        );
    }

    #[test]
    fn each_plan_store_keeps_at_most_max_plans_and_drops_the_oldest() {
        let store = PlanStore::new();
        let start = Instant::now();
        let at = |i: usize| start + Duration::from_millis(i as u64);
        let ids: Vec<String> = (0..=MAX_PLANS)
            .map(|i| store.put_send_at(1, "o1a", "mainnet", "o1b", 1, 0, json!({}), at(i)))
            .collect();
        let late = at(MAX_PLANS + 1);
        // The oldest plan was evicted; the newest MAX_PLANS still redeem.
        assert!(
            store
                .take_send_at(&ids[0], "o1b", 1, 0, 1, "o1a", "mainnet", None, late)
                .is_err()
        );
        for id in &ids[1..] {
            assert!(
                store
                    .take_send_at(id, "o1b", 1, 0, 1, "o1a", "mainnet", None, late)
                    .is_ok()
            );
        }
        let plan =
            json!({"selected_input_slots": [1], "fee_micronoid": 1, "output_value_micronoid": 1});
        let ids: Vec<String> = (0..=MAX_PLANS)
            .map(|i| store.put_consolidation_at(&plan, at(i)).unwrap())
            .collect();
        assert!(store.take_consolidation_at(&ids[0], late).is_err());
        assert!(store.take_consolidation_at(&ids[MAX_PLANS], late).is_ok());
        let preview = json!({"plan_id": "p", "fee_micronoid": "1", "noid_debit_micronoid": "0"});
        let ids: Vec<String> = (0..=MAX_PLANS)
            .map(|i| store.put_native_at(json!({}), &preview, at(i)).unwrap())
            .collect();
        assert!(store.take_native_at(&ids[0], &json!({}), late).is_err());
        assert!(
            store
                .take_native_at(&ids[MAX_PLANS], &json!({}), late)
                .is_ok()
        );
    }

    #[test]
    fn confirmation_strings_are_exact() {
        assert_eq!(confirm_send("o1abc", 42), "SEND 42 MICRONOID TO o1abc");
        assert_eq!(confirm_stop(258_000), "STOP AT HEIGHT 258000");
        assert_eq!(confirm_set_active(3), "SET ACTIVE ADDRESS 3");
    }
}
