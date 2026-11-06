// SPDX-License-Identifier: Apache-2.0
//! Live detection of the node's contract and native capabilities.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::manifest::Gate;
use crate::rpc::{RpcClient, RpcError};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    /// "2.1.x", "2.0.x" or "unknown" (capability-based, not a version RPC).
    pub detected: String,
    pub native_supported: bool,
    pub native_active: bool,
    pub native_activation_height: Option<u64>,
    pub native_tip_height: Option<u64>,
    pub native_runtime_available: bool,
    pub contract_runtime: bool,
    pub checked_height: Option<u64>,
    pub note: String,
    pub errors: Vec<String>,
    /// Raw probe responses reused by `node_overview`; not serialized here.
    #[serde(skip)]
    pub chain_info_raw: Option<Value>,
    #[serde(skip)]
    pub native_protocol_raw: Option<Value>,
    #[serde(skip)]
    pub contract_protocol_raw: Option<Value>,
}

impl Capabilities {
    /// Why a gated method is refused, or `None`. A refusal caused by a probe
    /// that failed says so, instead of describing the node as one without the
    /// capability.
    pub fn gate_error(&self, gate: Gate) -> Option<String> {
        let error = self.capability_error(gate)?;
        if self.errors.is_empty() {
            Some(error)
        } else {
            Some(format!(
                "could not read the node's capabilities ({}); refusing until the probe succeeds",
                self.errors.join("; ")
            ))
        }
    }

    fn capability_error(&self, gate: Gate) -> Option<String> {
        match gate {
            Gate::None => None,
            Gate::ContractRuntime if !self.contract_runtime => Some(
                "contract runtime is not available on this node (`getContractProtocol.runtime_available` is false)"
                    .into(),
            ),
            Gate::NativeSupported if !self.native_supported => Some(
                "native v2.1 RPC is not available on this node (v2.0.x has no native layer)".into(),
            ),
            Gate::NativeActive if !self.native_supported => Some(
                "native v2.1 RPC is not available on this node (v2.0.x has no native layer)".into(),
            ),
            Gate::NativeActive if !self.native_active => Some(format!(
                "native v2.1 is not active yet: activation height {}, current tip {}; native writes unlock there",
                self.native_activation_height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                self.native_tip_height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "unknown".into()),
            )),
            _ => None,
        }
    }
}

pub async fn probe(rpc: &RpcClient) -> Capabilities {
    let mut errors = Vec::new();

    let mut probe_results = rpc
        .batch_read(
            &[
                ("getChainInfo", Vec::new()),
                ("getNativeProtocol", Vec::new()),
                ("getContractProtocol", Vec::new()),
            ],
            PROBE_TIMEOUT,
        )
        .await;
    let contract = probe_results
        .pop()
        .unwrap_or_else(|| Err(RpcError::Invalid("probe response missing".into())));
    let native = probe_results
        .pop()
        .unwrap_or_else(|| Err(RpcError::Invalid("probe response missing".into())));
    let chain = probe_results
        .pop()
        .unwrap_or_else(|| Err(RpcError::Invalid("probe response missing".into())));

    // With no node at the URL all three calls fail with the same error: report
    // it once, as the actionable "no node answered" sentence, not three times.
    if let Err(error) = &chain
        && error.is_unreachable()
    {
        let message = error.to_string();
        return Capabilities {
            detected: "unknown".into(),
            native_supported: false,
            native_active: false,
            native_activation_height: None,
            native_tip_height: None,
            native_runtime_available: false,
            contract_runtime: false,
            checked_height: None,
            note: message.clone(),
            errors: vec![message],
            chain_info_raw: None,
            native_protocol_raw: None,
            contract_protocol_raw: None,
        };
    }

    let chain_info_raw = chain.as_ref().ok().cloned();
    let checked_height = match &chain {
        Ok(value) => value.get("height").and_then(Value::as_u64),
        Err(error) => {
            errors.push(format!("getChainInfo: {error}"));
            None
        }
    };

    let native_protocol_raw = native.as_ref().ok().cloned();
    let (
        native_supported,
        native_active,
        native_activation_height,
        native_tip_height,
        native_runtime_available,
    ) = match native {
        Ok(value) => {
            let tip = value
                .get("tip_height")
                .and_then(Value::as_u64)
                .or(checked_height);
            let activation = value.get("activation_height").and_then(Value::as_u64);
            let active_next = value
                .get("active_at_next_block")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let runtime = value
                .get("runtime_available")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reached = match (tip, activation) {
                (Some(tip), Some(activation)) => tip >= activation,
                _ => false,
            };
            (
                true,
                runtime && (active_next || reached),
                activation,
                tip,
                runtime,
            )
        }
        Err(error) if error.is_method_not_found() => (false, false, None, checked_height, false),
        Err(error) => {
            errors.push(format!("getNativeProtocol: {error}"));
            (false, false, None, checked_height, false)
        }
    };

    let contract_protocol_raw = contract.as_ref().ok().cloned();
    let (contract_ok, contract_runtime) = match contract {
        Ok(value) => {
            let runtime = value
                .get("runtime_available")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (true, runtime)
        }
        Err(error) if error.is_method_not_found() => (false, false),
        Err(error) => {
            errors.push(format!("getContractProtocol: {error}"));
            (false, false)
        }
    };

    let detected = if native_supported {
        "2.1.x"
    } else if contract_ok {
        "2.0.x"
    } else {
        "unknown"
    }
    .to_string();

    let note = if native_supported {
        if native_active {
            "native v2.1 is active".to_string()
        } else {
            format!(
                "native v2.1 is coded but inactive until height {} (tip {}); native writes are locked",
                native_activation_height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                native_tip_height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "unknown".into()),
            )
        }
    } else {
        "node does not expose native v2.1 RPC; native tools are unavailable".to_string()
    };

    Capabilities {
        detected,
        native_supported,
        native_active,
        native_activation_height,
        native_tip_height,
        native_runtime_available,
        contract_runtime,
        checked_height,
        note,
        errors,
        chain_info_raw,
        native_protocol_raw,
        contract_protocol_raw,
    }
}

pub struct CapabilityCache {
    rpc: Arc<RpcClient>,
    ttl: Duration,
    inner: Mutex<Option<(Instant, Capabilities)>>,
}

impl CapabilityCache {
    pub fn new(rpc: Arc<RpcClient>) -> Self {
        Self {
            rpc,
            ttl: Duration::from_secs(60),
            inner: Mutex::new(None),
        }
    }

    pub async fn get(&self, force: bool) -> Capabilities {
        if !force {
            let cached = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some((at, capabilities)) = cached
                && at.elapsed() < self.ttl
            {
                return capabilities;
            }
        }
        let capabilities = probe(&self.rpc).await;
        // Cache only a complete probe, so a node that was briefly down is
        // seen again on the next call instead of 60 s later.
        if capabilities.errors.is_empty() {
            *self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some((Instant::now(), capabilities.clone()));
        }
        capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(native_supported: bool, native_active: bool) -> Capabilities {
        Capabilities {
            detected: if native_supported { "2.1.x" } else { "2.0.x" }.into(),
            native_supported,
            native_active,
            native_activation_height: Some(279_370),
            native_tip_height: Some(258_000),
            native_runtime_available: native_supported,
            contract_runtime: true,
            checked_height: Some(258_000),
            note: String::new(),
            errors: Vec::new(),
            chain_info_raw: None,
            native_protocol_raw: None,
            contract_protocol_raw: None,
        }
    }

    #[test]
    fn native_write_is_locked_before_activation() {
        let error = caps(true, false).gate_error(Gate::NativeActive).unwrap();
        assert!(error.contains("279370"));
        assert!(caps(true, true).gate_error(Gate::NativeActive).is_none());
    }

    #[test]
    fn v204_has_no_native_layer() {
        let error = caps(false, false)
            .gate_error(Gate::NativeSupported)
            .unwrap();
        assert!(error.contains("v2.0.x"));
    }

    #[test]
    fn a_failed_probe_is_reported_as_a_failed_probe() {
        // The node did not answer; that is not proof it is a v2.0.x node.
        let mut capabilities = caps(false, false);
        capabilities.contract_runtime = false;
        capabilities.errors = vec!["getNativeProtocol: transport: connection refused".into()];
        for gate in [
            Gate::NativeSupported,
            Gate::NativeActive,
            Gate::ContractRuntime,
        ] {
            let error = capabilities.gate_error(gate).expect("still refused");
            assert!(error.contains("could not read"), "{error}");
            assert!(error.contains("connection refused"), "{error}");
            assert!(!error.contains("v2.0.x"), "{error}");
        }
        assert!(capabilities.gate_error(Gate::None).is_none());
    }

    #[tokio::test]
    async fn a_failed_probe_is_not_cached() {
        // Port 1 on loopback is closed: every probe fails fast.
        let rpc = Arc::new(RpcClient::new("http://127.0.0.1:1").expect("client"));
        let cache = CapabilityCache::new(rpc);
        let capabilities = cache.get(false).await;
        assert!(!capabilities.errors.is_empty());
        // No node at all: one actionable error, not one per probe call, and a
        // gated tool refuses with the same sentence.
        assert_eq!(capabilities.errors.len(), 1, "{:?}", capabilities.errors);
        assert!(
            capabilities.errors[0].starts_with(
                "No Parano1d node answered at http://127.0.0.1:1. Install and start one: https://parano1d.org (Downloads), then retry."
            ),
            "{:?}",
            capabilities.errors
        );
        let refusal = capabilities
            .gate_error(Gate::NativeSupported)
            .expect("refused");
        assert!(refusal.contains("No Parano1d node answered"), "{refusal}");
        assert!(
            cache.inner.lock().unwrap().is_none(),
            "a failed probe was cached"
        );
    }

    #[test]
    fn contract_gate_follows_runtime() {
        let mut capabilities = caps(true, true);
        capabilities.contract_runtime = false;
        assert!(capabilities.gate_error(Gate::ContractRuntime).is_some());
    }
}
