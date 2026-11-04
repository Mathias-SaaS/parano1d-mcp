// SPDX-License-Identifier: Apache-2.0
//! CLI smoke tests; no node is needed.

use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_parano1d-mcp"))
}

#[test]
fn version_flag_prints_version() {
    let output = binary().arg("--version").output().expect("run --version");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.starts_with("parano1d-mcp "), "{stdout}");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "{stdout}");
}

#[test]
fn help_flag_documents_modes_and_loopback() {
    let output = binary().arg("--help").output().expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("USAGE:"), "{stdout}");
    assert!(stdout.contains("PARANOID_MCP_MODE"), "{stdout}");
    assert!(stdout.contains("127.0.0.1"), "{stdout}");
}

#[test]
fn unknown_flag_exits_with_usage_error() {
    let output = binary().arg("--nope").output().expect("run --nope");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("USAGE:"), "{stderr}");
}
