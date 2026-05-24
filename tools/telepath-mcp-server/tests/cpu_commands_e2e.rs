mod helpers;

use heapless::Vec as HVec;
use helpers::{make_pair, spawn_fw};
use serde_json::json;
use telepath_client::TelepathClient;
use telepath_mcp_server::bridge;
use telepath_server::{command, TelepathServer};

// ---------------------------------------------------------------------------
// Stub CPU-only commands — same signatures as loopback-demo and nrf52840-ping.
// ---------------------------------------------------------------------------

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

#[command]
fn crc32(payload: HVec<u8, 128>) -> u32 {
    crc32_iso_hdlc(&payload)
}

#[command]
fn echo(payload: HVec<u8, 128>) -> HVec<u8, 128> {
    payload
}

fn run_fw(fw_side: helpers::FwSide, running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let mut server = TelepathServer::<_, 512>::new(fw_side, telepath_server::commands());
    while running.load(std::sync::atomic::Ordering::Acquire) {
        server.poll();
        std::thread::yield_now();
    }
}

// ---------------------------------------------------------------------------
// Discovery: all three CPU commands appear in the registry
// ---------------------------------------------------------------------------
//
// Note: invoke_* tests below use bridge::invoke with a positional JSON array,
// which is the low-level interface.  The named-argument mapping exercised by
// TelepathMcpServer::call_tool() is tested in named_args_e2e.rs.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn discover_includes_add_crc32_echo() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    let n = client.discover().expect("discover");
    assert!(n >= 3, "expected at least 3 commands, got {n}");

    let names: Vec<String> = client
        .schema_cache()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    for cmd in ["add", "crc32", "echo"] {
        assert!(
            names.iter().any(|n| n == cmd),
            "command '{cmd}' not in discovery result"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn discover_populates_arg_names_for_add() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "add")
        .expect("add not in schema cache")
        .clone();

    assert_eq!(
        entry.arg_names,
        vec!["a".to_string(), "b".to_string()],
        "add(a, b) must expose named arg list via discovery"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn discover_populates_arg_names_for_single_arg_command() {
    // crc32 and echo are 1-arg; add is 2-arg.
    // Use crc32 (single arg "payload") to confirm single-arg extraction.
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "crc32")
        .expect("crc32 not in schema cache")
        .clone();

    assert_eq!(
        entry.arg_names,
        vec!["payload".to_string()],
        "crc32(payload) must expose single arg name via discovery"
    );
}

// ---------------------------------------------------------------------------
// add: invoke(2, 3) → 5
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_add_returns_sum() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "add")
        .expect("add not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(
        &mut client,
        entry.cmd_id,
        &args_schema,
        &ret_schema,
        &json!([2, 3]),
    )
    .await
    .expect("invoke add");

    assert_eq!(result, json!(5i32));
}

// ---------------------------------------------------------------------------
// crc32: invoke over 128 zero bytes → 0xC2A8FA9D
// (CRC-32/ISO-HDLC verified with Python: zlib.crc32(bytes(128)) & 0xFFFFFFFF)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_crc32_zero_payload() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "crc32")
        .expect("crc32 not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let zeros: Vec<serde_json::Value> = vec![json!(0u8); 128];
    let result = bridge::invoke(
        &mut client,
        entry.cmd_id,
        &args_schema,
        &ret_schema,
        &json!([zeros]),
    )
    .await
    .expect("invoke crc32");

    assert_eq!(result, json!(0xC2A8_FA9Du32), "crc32 over 128 zeros");
}

// ---------------------------------------------------------------------------
// echo: payload is returned unchanged
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_echo_round_trip() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "echo")
        .expect("echo not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let seq: Vec<serde_json::Value> = (0u8..128).map(|b| json!(b)).collect();
    let result = bridge::invoke(
        &mut client,
        entry.cmd_id,
        &args_schema,
        &ret_schema,
        &json!([seq]),
    )
    .await
    .expect("invoke echo");

    let expected: Vec<serde_json::Value> = (0u8..128).map(|b| json!(b)).collect();
    assert_eq!(
        result,
        json!(expected),
        "echo must return input bytes unchanged"
    );
}
