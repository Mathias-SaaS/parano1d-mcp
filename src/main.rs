// SPDX-License-Identifier: Apache-2.0
//! parano1d-mcp: a read-only-by-default MCP server for a local Parano1d node.

#![forbid(unsafe_code)]

mod capabilities;
mod config;
mod manifest;
mod rpc;
mod safety;
mod tools;
mod util;

use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::rpc::RpcClient;
use crate::tools::Parano1dServer;

const HELP: &str = "\
parano1d-mcp — MCP server for a local Parano1d node (stdio)

USAGE:
  parano1d-mcp              Start the MCP server on stdio
  parano1d-mcp --check      Probe the node and print capabilities, then exit
  parano1d-mcp --version    Print the version
  parano1d-mcp --help       Print this help

ENVIRONMENT:
  PARANOID_RPC_URL                 http://127.0.0.1:9601 (loopback IP only)
  PARANOID_MCP_MODE                read | wallet | admin (default: read)
  PARANOID_MCP_MAX_SEND_MICRONOID  required for any spend
  PARANOID_MCP_MAX_FEE_MICRONOID   optional fee cap
  PARANOID_MCP_MAX_RESULT_BYTES    default 524288
  PARANOID_MCP_ALLOW_RAW_INTENT    default off; enables tx_submit
  RUST_LOG                         warn (stderr only)
";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [flag] if flag == "--version" || flag == "-V" => {
            println!("parano1d-mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{HELP}");
            return Ok(());
        }
        [flag] if flag == "--check" => return check_node().await,
        _ => {
            eprintln!("{HELP}");
            std::process::exit(2);
        }
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let config = Config::from_env().map_err(|e| format!("configuration: {e}"))?;
    tracing::info!(
        rpc = %config.rpc_url,
        mode = config.mode.as_str(),
        "starting parano1d-mcp"
    );

    let server = Parano1dServer::new(config)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

async fn check_node() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().map_err(|e| format!("configuration: {e}"))?;
    let rpc = RpcClient::new(config.rpc_url.clone())?;
    let capabilities = capabilities::probe(&rpc).await;
    let report = serde_json::json!({
        "rpc_url": config.rpc_url,
        "mode": config.mode.as_str(),
        "capabilities": capabilities,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !capabilities.errors.is_empty() {
        // The report is on stdout; say what went wrong on stderr too, so a
        // person running `--check` sees it (e.g. where to get a node).
        for error in &capabilities.errors {
            eprintln!("{error}");
        }
        std::process::exit(1);
    }
    Ok(())
}
