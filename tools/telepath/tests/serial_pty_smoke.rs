#![cfg(all(feature = "mcp", feature = "serial", unix))]

use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

// Build `host-pty-server` so the binary exists in the workspace target dir.
// `tools/telepath` is workspace-excluded, so `cargo test` here does NOT
// automatically build workspace members.
fn ensure_host_pty_server_built() -> PathBuf {
    let ws = workspace_root();
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "-p", "host-pty-server"])
        .current_dir(&ws)
        .status()
        .expect("invoke cargo build -p host-pty-server");
    assert!(status.success(), "cargo build -p host-pty-server failed");
    // Respect CARGO_TARGET_DIR when set (common in CI caches); fall back to
    // the workspace-default `<repo>/target`.
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.join("target"));
    target_dir.join("debug/host-pty-server")
}

/// Exercises the full PTY → serial transport → MCP bridge path without hardware.
///
/// Spawns `host-pty-server` (the firmware-side stand-in) and connects
/// `telepath mcp --transport serial --port <slave>` to it, then drives the
/// resulting MCP server as a client to call the `ping` tool and assert the
/// expected `0xDEADBEEF` (3735928559) result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serial_pty_mcp_ping_round_trip() {
    let host_pty_bin = ensure_host_pty_server_built();
    assert!(
        host_pty_bin.is_file(),
        "host-pty-server not found at {host_pty_bin:?}"
    );

    // 1. Spawn host-pty-server and read HOST_PTY_SERVER_PATH= from its stdout.
    let mut server_proc = Command::new(&host_pty_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn host-pty-server");

    let stdout = server_proc.stdout.take().expect("host-pty-server stdout");
    let mut lines = BufReader::new(stdout).lines();

    let slave_path = timeout(Duration::from_secs(10), async {
        loop {
            match lines
                .next_line()
                .await
                .expect("read stdout from host-pty-server")
            {
                Some(line) => {
                    if let Some(path) = line.strip_prefix("HOST_PTY_SERVER_PATH=") {
                        return path.to_string();
                    }
                }
                None => panic!(
                    "host-pty-server stdout closed without printing HOST_PTY_SERVER_PATH"
                ),
            }
        }
    })
    .await
    .expect("timed out waiting for HOST_PTY_SERVER_PATH from host-pty-server");

    // 2. Spawn `telepath mcp --transport serial --port <slave>` as an MCP server
    //    driven via rmcp's TokioChildProcess client transport.
    let telepath_bin: &Path = Path::new(env!("CARGO_BIN_EXE_telepath"));
    let mut mcp_cmd = Command::new(telepath_bin);
    mcp_cmd.args(["mcp", "--transport", "serial", "--port", &slave_path]);

    let transport =
        TokioChildProcess::new(mcp_cmd).expect("spawn telepath mcp as TokioChildProcess");
    let client = ().serve(transport).await.expect("MCP initialize handshake");

    // 3. Confirm `ping` is among the discovered tools.
    let tools = client
        .list_tools(None)
        .await
        .expect("list_tools");
    assert!(
        tools.tools.iter().any(|t| t.name == "ping"),
        "ping not in discovered tools: {:?}",
        tools.tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>()
    );

    // 4. Call `ping` and assert the 0xDEADBEEF sentinel.
    let mut call_params = CallToolRequestParams::default();
    call_params.name = "ping".into();
    let result = client
        .call_tool(call_params)
        .await
        .expect("call_tool ping");

    let text = result
        .content
        .iter()
        .find_map(|c| c.raw.as_text().map(|t| t.text.clone()))
        .expect("ping result must contain a text content item");

    assert_eq!(
        text, "3735928559",
        "ping must return 0xDEADBEEF (3735928559)"
    );

    // 5. Shut down cleanly; wait() reaps the child to avoid zombie processes.
    client.cancel().await.ok();
    let _ = server_proc.kill().await;
    let _ = server_proc.wait().await;
}
