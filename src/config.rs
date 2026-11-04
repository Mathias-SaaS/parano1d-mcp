// SPDX-License-Identifier: Apache-2.0
//! Environment configuration. No secrets: the node RPC is loopback and unauthenticated.

use std::env;
use std::time::Duration;

use reqwest::Url;

use crate::manifest::Access;

pub const DEFAULT_RPC_URL: &str = "http://127.0.0.1:9601";
pub const DEFAULT_MAX_RESULT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Read,
    Wallet,
    Admin,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" | "ro" | "read-only" => Ok(Self::Read),
            "wallet" | "write" | "rw" => Ok(Self::Wallet),
            "admin" => Ok(Self::Admin),
            other => Err(format!(
                "invalid PARANOID_MCP_MODE {other:?}; use read, wallet or admin"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Wallet => "wallet",
            Self::Admin => "admin",
        }
    }

    pub fn allows(self, access: Access) -> bool {
        match access {
            Access::Read => true,
            Access::Wallet => self >= Self::Wallet,
            Access::Admin => self >= Self::Admin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub rpc_url: String,
    pub mode: Mode,
    pub max_send_micronoid: Option<u64>,
    pub max_fee_micronoid: Option<u64>,
    pub max_result_bytes: usize,
    pub request_timeout: Duration,
    /// Extra opt-in for `tx_submit`: raw signed intents bypass the plan/cap path.
    pub allow_raw_intent: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// `from_env` with the environment injected, so tests can drive every
    /// variable without `std::env::set_var` (unsafe in edition 2024, and this
    /// crate forbids unsafe). `None` means unset.
    fn from_lookup<F>(get: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let rpc_url = get("PARANOID_RPC_URL").unwrap_or_else(|| DEFAULT_RPC_URL.to_string());
        validate_loopback(&rpc_url)?;

        let mode = match get("PARANOID_MCP_MODE") {
            Some(value) => Mode::parse(&value)?,
            None => Mode::Read,
        };

        let max_send_micronoid = parse_optional_u64(&get, "PARANOID_MCP_MAX_SEND_MICRONOID")?;
        let max_fee_micronoid = parse_optional_u64(&get, "PARANOID_MCP_MAX_FEE_MICRONOID")?;
        let allow_raw_intent = parse_bool_env(&get, "PARANOID_MCP_ALLOW_RAW_INTENT")?;
        let max_result_bytes = match get("PARANOID_MCP_MAX_RESULT_BYTES") {
            Some(value) => value
                .parse::<usize>()
                .map_err(|e| format!("PARANOID_MCP_MAX_RESULT_BYTES: {e}"))?,
            None => DEFAULT_MAX_RESULT_BYTES,
        };
        if max_result_bytes < 1024 {
            return Err("PARANOID_MCP_MAX_RESULT_BYTES must be at least 1024".to_string());
        }

        Ok(Self {
            rpc_url,
            mode,
            max_send_micronoid,
            max_fee_micronoid,
            max_result_bytes,
            request_timeout: Duration::from_secs(60),
            allow_raw_intent,
        })
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        other => Err(format!(
            "invalid boolean {other:?}; use 1/0, true/false, yes/no or on/off"
        )),
    }
}

fn parse_bool_env<F>(get: &F, name: &str) -> Result<bool, String>
where
    F: Fn(&str) -> Option<String>,
{
    parse_bool_value(get(name).as_deref()).map_err(|e| format!("{name}: {e}"))
}

fn parse_bool_value(value: Option<&str>) -> Result<bool, String> {
    match value {
        Some(value) => parse_bool(value),
        None => Ok(false),
    }
}

fn parse_optional_u64<F>(get: &F, name: &str) -> Result<Option<u64>, String>
where
    F: Fn(&str) -> Option<String>,
{
    match get(name) {
        Some(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|e| format!("{name}: {e}")),
        None => Ok(None),
    }
}

/// This server is intentionally loopback-only. It has no credentials, so a remote
/// listener would be an unauthenticated owner-scope endpoint.
fn validate_loopback(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|e| format!("PARANOID_RPC_URL: {e}"))?;
    if url.scheme() != "http" {
        return Err("PARANOID_RPC_URL must use http:// on loopback".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("PARANOID_RPC_URL must not contain credentials".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "PARANOID_RPC_URL has no host".to_string())?;
    if !matches!(host, "127.0.0.1" | "::1" | "[::1]") {
        return Err(format!(
            "PARANOID_RPC_URL host {host:?} is not a loopback IP literal; use 127.0.0.1 or [::1]. \
             Hostnames such as localhost can be redirected and are refused."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_ordered() {
        assert!(Mode::Admin.allows(Access::Admin));
        assert!(Mode::Wallet.allows(Access::Wallet));
        assert!(!Mode::Wallet.allows(Access::Admin));
        assert!(Mode::Read.allows(Access::Read));
        assert!(!Mode::Read.allows(Access::Wallet));
    }

    #[test]
    fn rejects_remote_or_hostname_rpc() {
        assert!(validate_loopback("http://192.0.2.10:9601").is_err());
        assert!(validate_loopback("http://127.0.0.1:9601").is_ok());
        assert!(validate_loopback("http://[::1]:9601").is_ok());
        assert!(validate_loopback("http://localhost:19821").is_err());
        assert!(validate_loopback("https://127.0.0.1:9601").is_err());
        assert!(validate_loopback("http://user:pass@127.0.0.1:9601").is_err());
        // A username without a password must still be refused: credentials in
        // the URL are never acceptable.
        assert!(validate_loopback("http://user@127.0.0.1:9601").is_err());
    }

    #[test]
    fn parses_boolean_environment_values() {
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("TRUE").unwrap());
        assert!(parse_bool("yes").unwrap());
        assert!(!parse_bool("0").unwrap());
        assert!(!parse_bool("off").unwrap());
        assert!(!parse_bool("").unwrap());
        assert!(parse_bool("maybe").is_err());
    }

    #[test]
    fn an_unset_boolean_is_off_and_a_set_one_is_read() {
        // An unset variable must never enable an opt-in such as
        // PARANOID_MCP_ALLOW_RAW_INTENT.
        assert!(!parse_bool_value(None).unwrap());
        assert!(parse_bool_value(Some("1")).unwrap());
        assert!(parse_bool_value(Some("true")).unwrap());
        assert!(!parse_bool_value(Some("0")).unwrap());
        assert!(parse_bool_value(Some("maybe")).is_err());
    }

    #[test]
    fn mode_names_are_stable() {
        assert_eq!(Mode::Read.as_str(), "read");
        assert_eq!(Mode::Wallet.as_str(), "wallet");
        assert_eq!(Mode::Admin.as_str(), "admin");
    }

    #[test]
    fn the_default_result_limit_is_512_kib() {
        assert_eq!(DEFAULT_MAX_RESULT_BYTES, 524_288);
    }

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        }
    }

    #[test]
    fn configuration_is_read_from_the_environment() {
        let config = Config::from_lookup(lookup(&[
            ("PARANOID_MCP_MODE", "wallet"),
            ("PARANOID_MCP_MAX_SEND_MICRONOID", "500"),
            ("PARANOID_MCP_MAX_FEE_MICRONOID", "50"),
            ("PARANOID_MCP_ALLOW_RAW_INTENT", "1"),
            ("PARANOID_MCP_MAX_RESULT_BYTES", "4096"),
            ("PARANOID_RPC_URL", "http://[::1]:9601"),
        ]))
        .expect("valid environment");
        assert_eq!(config.mode, Mode::Wallet);
        assert_eq!(config.max_send_micronoid, Some(500));
        assert_eq!(config.max_fee_micronoid, Some(50));
        assert!(config.allow_raw_intent);
        assert_eq!(config.max_result_bytes, 4096);
        assert_eq!(config.rpc_url, "http://[::1]:9601");
    }

    #[test]
    fn an_empty_environment_fails_closed() {
        let config = Config::from_lookup(lookup(&[])).expect("defaults");
        // Read-only, no spend allowed, no raw intent, the documented URL.
        assert_eq!(config.mode, Mode::Read);
        assert_eq!(config.max_send_micronoid, None);
        assert_eq!(config.max_fee_micronoid, None);
        assert!(!config.allow_raw_intent);
        assert_eq!(config.max_result_bytes, DEFAULT_MAX_RESULT_BYTES);
        assert_eq!(config.rpc_url, DEFAULT_RPC_URL);
    }

    #[test]
    fn malformed_configuration_is_an_error_not_a_default() {
        for (name, value) in [
            ("PARANOID_MCP_MODE", "root"),
            ("PARANOID_MCP_MAX_SEND_MICRONOID", "abc"),
            ("PARANOID_MCP_MAX_SEND_MICRONOID", "-1"),
            ("PARANOID_MCP_MAX_FEE_MICRONOID", "1.5"),
            ("PARANOID_MCP_ALLOW_RAW_INTENT", "maybe"),
            ("PARANOID_MCP_MAX_RESULT_BYTES", "1023"),
            ("PARANOID_MCP_MAX_RESULT_BYTES", "lots"),
            ("PARANOID_RPC_URL", "http://192.0.2.10:9601"),
        ] {
            assert!(
                Config::from_lookup(lookup(&[(name, value)])).is_err(),
                "{name}={value} was accepted"
            );
        }
        // The lower bound of the result limit is inclusive.
        assert!(Config::from_lookup(lookup(&[("PARANOID_MCP_MAX_RESULT_BYTES", "1024")])).is_ok());
    }
}
