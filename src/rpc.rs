// SPDX-License-Identifier: Apache-2.0
//! Minimal JSON-RPC 2.0 client for the local Parano1d node.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{Value, json};

const RETRY_DELAY: Duration = Duration::from_millis(250);
/// The node's documented HTTP request-body limit.
const MAX_RPC_REQUEST_BYTES: usize = 2_237_632;
/// Generous bound for a loopback response; the largest known response is a
/// contract receipt near 1.1 MB.
const MAX_RPC_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, thiserror::Error)]
pub enum RpcError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("JSON-RPC {code}: {message}")]
    JsonRpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("invalid response: {0}")]
    Invalid(String),
    #[error("request exceeds {0} bytes")]
    RequestTooLarge(usize),
}

impl RpcError {
    pub fn is_method_not_found(&self) -> bool {
        matches!(self, Self::JsonRpc { code: -32601, .. })
    }

    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Invalid(_) => true,
            Self::Http { status, .. } => *status >= 500,
            Self::JsonRpc { .. } | Self::RequestTooLarge(_) => false,
        }
    }

    /// True when the failure may have happened after the node acted on the
    /// request: the request was sent but no usable response came back. A
    /// JSON-RPC error is a definite rejection (the node answered), an HTTP 4xx
    /// is the transport refusing the request, and a too-large request is
    /// refused before it is sent, so none of those are ambiguous.
    pub fn outcome_unknown(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Invalid(_) => true,
            Self::Http { status, .. } => *status >= 500,
            Self::JsonRpc { .. } | Self::RequestTooLarge(_) => false,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Transport(message) => json!({"kind": "transport", "message": message}),
            Self::Http { status, body } => {
                json!({"kind": "http", "status": status, "body": body})
            }
            Self::JsonRpc {
                code,
                message,
                data,
            } => json!({"kind": "json_rpc", "code": code, "message": message, "data": data}),
            Self::Invalid(message) => json!({"kind": "invalid_response", "message": message}),
            Self::RequestTooLarge(limit) => {
                json!({"kind": "request_too_large", "limit_bytes": limit})
            }
        }
    }
}

pub struct RpcClient {
    url: String,
    http: reqwest::Client,
    next_id: AtomicU64,
}

impl RpcClient {
    pub fn new(url: impl Into<String>) -> Result<Self, String> {
        let url = url.into();
        // The RPC is a loopback IP literal. Do not let proxy environment
        // variables or an HTTP redirect move the request off the loopback
        // trust boundary.
        let http = reqwest::Client::builder()
            .user_agent(concat!("parano1d-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("building HTTP client: {e}"))?;
        Ok(Self {
            url,
            http,
            next_id: AtomicU64::new(1),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Call one method. `retry` must be false for anything that can write.
    pub async fn call(
        &self,
        method: &str,
        params: Vec<Value>,
        timeout: Duration,
        retry: bool,
    ) -> Result<Value, RpcError> {
        let attempts = if retry { 2 } else { 1 };
        let mut last_error = None;
        for attempt in 0..attempts {
            match self.call_once(method, params.clone(), timeout).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let can_retry = retry && attempt + 1 < attempts && error.is_retryable();
                    if !can_retry {
                        return Err(error);
                    }
                    last_error = Some(error);
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| RpcError::Invalid("request failed".into())))
    }

    async fn call_once(
        &self,
        method: &str,
        params: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": rpc_name(method),
            "params": params,
        });
        let value = self.post(body, timeout).await?;
        parse_single(value, Some(id))
    }

    /// Batch read-only calls, retrying once if every result was transient.
    pub async fn batch_read(
        &self,
        calls: &[(&str, Vec<Value>)],
        timeout: Duration,
    ) -> Vec<Result<Value, RpcError>> {
        let results = self.batch_once(calls, timeout).await;
        if should_retry_batch(&results) {
            tokio::time::sleep(RETRY_DELAY).await;
            self.batch_once(calls, timeout).await
        } else {
            results
        }
    }

    async fn batch_once(
        &self,
        calls: &[(&str, Vec<Value>)],
        timeout: Duration,
    ) -> Vec<Result<Value, RpcError>> {
        if calls.is_empty() {
            return Vec::new();
        }
        let base = self
            .next_id
            .fetch_add(calls.len() as u64, Ordering::Relaxed);
        let requests: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(index, (method, params))| {
                json!({
                    "jsonrpc": "2.0",
                    "id": base + index as u64,
                    "method": rpc_name(method),
                    "params": params,
                })
            })
            .collect();

        match self.post(Value::Array(requests), timeout).await {
            Ok(response) => match_batch(response, base, calls),
            Err(error) => calls.iter().map(|_| Err(error.clone())).collect(),
        }
    }
}

/// Match a batch response to its requests by id. Ids outside
/// `base..base + calls.len()` are ignored, and a request without a reply is an
/// error, so a reordered, short or padded batch can never shift a result onto
/// the wrong method.
fn match_batch(
    response: Value,
    base: u64,
    calls: &[(&str, Vec<Value>)],
) -> Vec<Result<Value, RpcError>> {
    let Value::Array(items) = response else {
        let error = RpcError::Invalid("batch response was not an array".into());
        return calls.iter().map(|_| Err(error.clone())).collect();
    };

    let mut results: Vec<Option<Result<Value, RpcError>>> =
        (0..calls.len()).map(|_| None).collect();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(offset) = id
            .checked_sub(base)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        if offset < results.len() {
            results[offset] = Some(parse_single(item, Some(id)));
        }
    }

    results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            result.unwrap_or_else(|| {
                Err(RpcError::Invalid(format!(
                    "batch response missing id for {}",
                    calls[index].0
                )))
            })
        })
        .collect()
}

impl RpcClient {
    async fn post(&self, body: Value, timeout: Duration) -> Result<Value, RpcError> {
        let started = Instant::now();
        let bytes = encode_request(&body)?;
        let response = self
            .http
            .post(&self.url)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|e| RpcError::Transport(format!("{e}")))?;
        let status = response.status().as_u16();
        if let Some(length) = response.content_length() {
            ensure_response_size(usize::try_from(length).unwrap_or(usize::MAX))?;
        }
        // Stream the body with a hard cap. The node is loopback, but the port
        // is unauthenticated: checking the size only after `text()` buffers
        // the whole body would let a misbehaving local process grow memory
        // without bound (or lie about content-length).
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| RpcError::Transport(format!("reading response: {e}")))?;
            push_checked(&mut body, &chunk)?;
        }
        tracing::debug!(
            status,
            elapsed_ms = started.elapsed().as_millis(),
            "RPC HTTP"
        );
        decode_response(status, body)
    }
}

/// Turn an HTTP status and a complete (already size-capped) body into a JSON
/// value or an error.
fn decode_response(status: u16, body: Vec<u8>) -> Result<Value, RpcError> {
    let text = String::from_utf8(body)
        .map_err(|e| RpcError::Invalid(format!("response was not UTF-8: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(RpcError::Http {
            status,
            body: truncate(&text, 500),
        });
    }
    serde_json::from_str(&text)
        .map_err(|e| RpcError::Invalid(format!("{e}: {}", truncate(&text, 200))))
}

fn encode_request(body: &Value) -> Result<Vec<u8>, RpcError> {
    let bytes = serde_json::to_vec(body)
        .map_err(|e| RpcError::Invalid(format!("encoding request: {e}")))?;
    if bytes.len() > MAX_RPC_REQUEST_BYTES {
        return Err(RpcError::RequestTooLarge(MAX_RPC_REQUEST_BYTES));
    }
    Ok(bytes)
}

fn ensure_response_size(length: usize) -> Result<(), RpcError> {
    if length > MAX_RPC_RESPONSE_BYTES {
        Err(RpcError::Invalid(format!(
            "response exceeds {MAX_RPC_RESPONSE_BYTES} bytes"
        )))
    } else {
        Ok(())
    }
}

/// Append one streamed chunk, refusing to cross the response cap. The cap is
/// enforced per chunk, so an oversized stream stops early instead of growing
/// a buffer past the limit.
fn push_checked(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), RpcError> {
    let joined = body.len().saturating_add(chunk.len());
    if joined > MAX_RPC_RESPONSE_BYTES {
        return Err(RpcError::Invalid(format!(
            "response exceeds {MAX_RPC_RESPONSE_BYTES} bytes"
        )));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn should_retry_batch(results: &[Result<Value, RpcError>]) -> bool {
    !results.is_empty()
        && results
            .iter()
            .all(|result| matches!(result, Err(error) if error.is_retryable()))
}

fn rpc_name(method: &str) -> String {
    if method.starts_with("paranoid_") {
        method.to_string()
    } else {
        format!("paranoid_{method}")
    }
}

fn parse_single(value: Value, expected_id: Option<u64>) -> Result<Value, RpcError> {
    let object = value
        .as_object()
        .ok_or_else(|| RpcError::Invalid("response was not an object".into()))?;
    if let Some(expected) = expected_id {
        let actual = object.get("id").and_then(Value::as_u64);
        if actual != Some(expected) {
            return Err(RpcError::Invalid(format!(
                "response id mismatch: expected {expected}, got {actual:?}"
            )));
        }
    }
    if let Some(error) = object.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown JSON-RPC error")
            .to_string();
        let data = error.get("data").cloned();
        return Err(RpcError::JsonRpc {
            code,
            message,
            data,
        });
    }
    object
        .get("result")
        .cloned()
        .ok_or_else(|| RpcError::Invalid("response has neither result nor error".into()))
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        let end = text
            .char_indices()
            .map(|(index, ch)| index + ch.len_utf8())
            .take_while(|end| *end <= max)
            .last()
            .unwrap_or(0);
        format!("{}…", &text[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_namespace_once() {
        assert_eq!(rpc_name("getChainInfo"), "paranoid_getChainInfo");
        assert_eq!(rpc_name("paranoid_getChainInfo"), "paranoid_getChainInfo");
    }

    #[test]
    fn parses_result_and_error() {
        assert_eq!(
            parse_single(json!({"jsonrpc":"2.0","id":1,"result":42}), Some(1)).unwrap(),
            json!(42)
        );
        let error = parse_single(
            json!({
                "jsonrpc":"2.0","id":1,
                "error":{"code":-32601,"message":"Method not found"}
            }),
            Some(1),
        )
        .unwrap_err();
        assert!(error.is_method_not_found());
    }

    #[test]
    fn rejects_malformed_response() {
        assert!(parse_single(json!({"jsonrpc":"2.0","id":1}), Some(1)).is_err());
        assert!(parse_single(json!([]), None).is_err());
    }

    #[test]
    fn a_missing_error_code_defaults_to_minus_32000() {
        let error = parse_single(
            json!({"jsonrpc":"2.0","id":1,"error":{"message":"x"}}),
            Some(1),
        )
        .unwrap_err();
        assert!(
            matches!(error, RpcError::JsonRpc { code: -32000, .. }),
            "{error}"
        );
    }

    #[test]
    fn rejects_response_id_mismatch() {
        let error = parse_single(json!({"jsonrpc":"2.0","id":2,"result":42}), Some(1)).unwrap_err();
        assert!(matches!(error, RpcError::Invalid(_)));
        assert!(parse_single(json!({"jsonrpc":"2.0","result":42}), Some(1)).is_err());
    }

    #[test]
    fn caps_request_and_response_sizes() {
        assert!(encode_request(&json!({"a": 1})).is_ok());
        assert!(encode_request(&json!({"a": "x".repeat(MAX_RPC_REQUEST_BYTES)})).is_err());
        assert!(ensure_response_size(1024).is_ok());
        assert!(ensure_response_size(MAX_RPC_RESPONSE_BYTES + 1).is_err());
    }

    #[test]
    fn size_cap_boundaries_are_exact() {
        // Pin the constants themselves: a mutated constant would otherwise
        // silently change every boundary below.
        assert_eq!(MAX_RPC_REQUEST_BYTES, 2_237_632);
        assert_eq!(MAX_RPC_RESPONSE_BYTES, 16_777_216);

        // Exactly at the cap is allowed; one byte over is refused.
        assert!(ensure_response_size(MAX_RPC_RESPONSE_BYTES).is_ok());
        assert!(ensure_response_size(MAX_RPC_RESPONSE_BYTES + 1).is_err());

        let at_cap = json!({"a": "x".repeat(MAX_RPC_REQUEST_BYTES - 8)});
        let encoded = encode_request(&at_cap).expect("a body at the cap is allowed");
        assert_eq!(encoded.len(), MAX_RPC_REQUEST_BYTES);
        let over = json!({"a": "x".repeat(MAX_RPC_REQUEST_BYTES)});
        assert!(encode_request(&over).is_err());
    }

    #[test]
    fn method_not_found_is_exact_and_http_retry_is_5xx_only() {
        let not_found = RpcError::JsonRpc {
            code: -32601,
            message: "Method not found".into(),
            data: None,
        };
        assert!(not_found.is_method_not_found());
        for code in [-32600, -32000, 0] {
            let other = RpcError::JsonRpc {
                code,
                message: "other".into(),
                data: None,
            };
            assert!(!other.is_method_not_found(), "code {code}");
        }
        assert!(
            !RpcError::Http {
                status: 400,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            !RpcError::Http {
                status: 499,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            RpcError::Http {
                status: 500,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(RpcError::Transport("x".into()).is_retryable());
        assert!(!not_found.is_retryable());
    }

    #[test]
    fn error_json_and_url_are_reported() {
        let client = RpcClient::new("http://127.0.0.1:9601").expect("client");
        assert_eq!(client.url(), "http://127.0.0.1:9601");
        let error = RpcError::Http {
            status: 503,
            body: "down".into(),
        };
        let value = error.to_json();
        assert_eq!(value["kind"], "http");
        assert_eq!(value["status"], 503);
        let json_rpc = RpcError::JsonRpc {
            code: -32000,
            message: "m".into(),
            data: None,
        };
        assert_eq!(json_rpc.to_json()["kind"], "json_rpc");
    }

    #[test]
    fn streamed_chunks_are_capped_incrementally() {
        let mut body = Vec::new();
        assert!(push_checked(&mut body, &[0u8; 1024]).is_ok());
        assert_eq!(body.len(), 1024);

        // A chunk that would cross the cap is refused and the buffer is
        // left unchanged, so the early error stops the stream.
        let mut body = Vec::new();
        assert!(push_checked(&mut body, &[0u8; MAX_RPC_RESPONSE_BYTES]).is_ok());
        assert!(push_checked(&mut body, &[0u8; 1]).is_err());
        assert_eq!(body.len(), MAX_RPC_RESPONSE_BYTES);

        // Oversized single chunks are refused outright.
        let mut body = Vec::new();
        assert!(push_checked(&mut body, &[0u8; MAX_RPC_RESPONSE_BYTES + 1]).is_err());
        assert!(body.is_empty());
    }

    /// A peer that accepts the request and then stalls must hit the client
    /// timeout instead of hanging the tool. The real MCP uses 60 s; the test
    /// passes a short timeout so CI stays fast.
    #[tokio::test]
    async fn a_stalled_peer_hits_the_request_timeout() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stalled peer");
        let addr = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        let client = RpcClient::new(format!("http://{addr}")).expect("client");
        let started = Instant::now();
        let error = client
            .call("blockCount", vec![], Duration::from_millis(250), false)
            .await
            .expect_err("a stalled peer must not succeed");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout did not fire: {:?}",
            started.elapsed()
        );
        assert!(matches!(error, RpcError::Transport(_)), "{error}");
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("éé", 3), "é…");
        assert_eq!(truncate("éé", 2), "é…");
        assert_eq!(truncate("éé", 1), "…");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn batch_retries_only_when_every_result_is_transient() {
        let transient = vec![
            Err(RpcError::Transport("a".into())),
            Err(RpcError::Transport("b".into())),
        ];
        assert!(should_retry_batch(&transient));

        let mixed = vec![
            Err(RpcError::Transport("a".into())),
            Err(RpcError::JsonRpc {
                code: -32601,
                message: "Method not found".into(),
                data: None,
            }),
        ];
        assert!(!should_retry_batch(&mixed));
        assert!(!should_retry_batch(&[]));
    }
}
