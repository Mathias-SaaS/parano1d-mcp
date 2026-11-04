// SPDX-License-Identifier: Apache-2.0
//! Peer reporting: everything the node exposes, and an explicit account of what it does not.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Parano1dServer, rpc_error_string};
use crate::rpc::RpcError;

impl Parano1dServer {
    pub(crate) async fn peer_report_impl(&self) -> Result<String, String> {
        let calls = [
            ("getPeerCount", vec![]),
            ("getNodeStatus", vec![]),
            ("getChainInfo", vec![]),
            ("getMiningInfo", vec![]),
        ];
        let mut results = self
            .rpc
            .batch_read(&calls, Duration::from_secs(20))
            .await
            .into_iter();
        let peer_count = results
            .next()
            .unwrap_or_else(|| Err(RpcError::Invalid("peer report response missing".into())))
            .map_err(rpc_error_string);
        let node_status = results
            .next()
            .unwrap_or_else(|| Err(RpcError::Invalid("peer report response missing".into())))
            .map_err(rpc_error_string);
        let chain_info = results
            .next()
            .unwrap_or_else(|| Err(RpcError::Invalid("peer report response missing".into())))
            .map_err(rpc_error_string);
        let mining_info = results
            .next()
            .unwrap_or_else(|| Err(RpcError::Invalid("peer report response missing".into())))
            .map_err(rpc_error_string);
        self.capped(build_peer_report(
            peer_count,
            node_status,
            chain_info,
            mining_info,
        ))
    }
}

fn build_peer_report(
    peer_count: Result<Value, String>,
    node_status: Result<Value, String>,
    chain_info: Result<Value, String>,
    mining_info: Result<Value, String>,
) -> Value {
    let summary = match &node_status {
        Ok(node) => {
            let field = |name: &str| node.get(name).cloned().unwrap_or(Value::Null);
            let connected = node
                .get("p2p_connected_peers")
                .cloned()
                .or_else(|| peer_count.as_ref().ok().cloned())
                .unwrap_or(Value::Null);
            let height = chain_info
                .as_ref()
                .ok()
                .and_then(|chain| chain.get("height"))
                .cloned()
                .or_else(|| {
                    mining_info
                        .as_ref()
                        .ok()
                        .and_then(|mining| mining.get("height"))
                        .cloned()
                })
                .unwrap_or(Value::Null);
            json!({
                "connected_peers": connected,
                "dispatchable_peers": field("p2p_dispatchable_peers"),
                "mining_confirmed_peers": field("mining_confirmed_peers"),
                "mining_required_peers": field("mining_required_peers"),
                "isolated_mining": field("isolated_mining"),
                "relay_reservations": field("p2p_relay_reservations"),
                "heartbeat_age_ms": field("p2p_heartbeat_age_ms"),
                "healthy": field("p2p_healthy"),
                "testnet": field("testnet"),
                "height": height
            })
        }
        Err(error) => json!({"error": error}),
    };

    json!({
        "summary": summary,
        "peer_count": value_or_error(&peer_count),
        "node_status": value_or_error(&node_status),
        "chain_info": value_or_error(&chain_info),
        "mining_info": value_or_error(&mining_info),
        "per_peer_details": {
            "available": false,
            "reason": "The node's public JSON-RPC exposes aggregate peer counts and P2P health only. It has no per-peer RPC and no latency metric.",
            "what_would_be_needed": [
                "peer_id", "addresses", "direction", "connected_since", "last_seen", "latency_ms"
            ],
            "historical_cache": "~/.parano1d/data/peers.json (peer_id, up to 8 multiaddrs, last_success_unix) is a discovery hint, not a live peer list. This server does not read node files.",
            "historical_cache_enabled": false,
            "upstream": "A getPeers RPC is not yet available upstream; that RPC is the complete fix."
        },
        "note": "Counts move as peers connect and disconnect. No per-peer IP, port or latency is available through the node RPC."
    })
}

fn value_or_error(result: &Result<Value, String>) -> Value {
    match result {
        Ok(value) => value.clone(),
        Err(error) => json!({"error": error}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_honest_about_per_peer_data() {
        let report = build_peer_report(
            Ok(json!(10)),
            Ok(json!({
                "testnet": false,
                "p2p_connected_peers": 10,
                "p2p_dispatchable_peers": 9,
                "mining_confirmed_peers": 8,
                "mining_required_peers": 1,
                "isolated_mining": false,
                "p2p_relay_reservations": 2,
                "p2p_heartbeat_age_ms": 188,
                "p2p_healthy": true
            })),
            Ok(json!({"height": 258_647})),
            Ok(json!({"height": 258_647})),
        );
        assert_eq!(report["summary"]["connected_peers"], 10);
        assert_eq!(report["summary"]["dispatchable_peers"], 9);
        assert_eq!(report["summary"]["height"], 258_647);
        assert_eq!(report["per_peer_details"]["available"], false);
        assert_eq!(
            report["per_peer_details"]["historical_cache_enabled"],
            false
        );
    }

    #[test]
    fn report_preserves_rpc_errors() {
        let report = build_peer_report(
            Err("node down".into()),
            Err("node down".into()),
            Err("node down".into()),
            Err("node down".into()),
        );
        assert_eq!(report["peer_count"]["error"], "node down");
        assert_eq!(report["summary"]["error"], "node down");
    }
}
