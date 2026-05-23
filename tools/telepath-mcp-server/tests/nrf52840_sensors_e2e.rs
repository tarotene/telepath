mod helpers;

use helpers::{make_pair, spawn_fw};
use serde_json::json;
use telepath_client::TelepathClient;
use telepath_mcp_server::bridge;
use telepath_server::{command, TelepathServer};

// ---------------------------------------------------------------------------
// Stub sensor commands — same signatures as nrf52840-ping will expose.
// ---------------------------------------------------------------------------

#[command]
fn temp_read() -> i16 {
    100 // 25 °C in 0.25 °C units
}

#[command]
fn rng_u32() -> u32 {
    0xCAFE_BABE
}

#[command]
fn ficr_uid() -> (u32, u32) {
    (0xAAAA_AAAA, 0x5555_5555)
}

#[command]
fn saadc_vdd_mv() -> u16 {
    3300
}

fn run_fw(fw_side: helpers::FwSide, running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let mut server = TelepathServer::<_, 512>::new(fw_side, telepath_server::commands());
    while running.load(std::sync::atomic::Ordering::Acquire) {
        server.poll();
        std::thread::yield_now();
    }
}

// ---------------------------------------------------------------------------
// Discovery: all four commands appear with correct names
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn discover_all_sensor_commands() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    let n = client.discover().expect("discover");
    assert!(n >= 4, "expected at least 4 commands, got {n}");

    let names: Vec<String> = client.schema_cache().iter().map(|e| e.name.clone()).collect();
    for cmd in ["temp_read", "rng_u32", "ficr_uid", "saadc_vdd_mv"] {
        assert!(names.iter().any(|n| n == cmd), "command '{cmd}' not in discovery result");
    }
}

// ---------------------------------------------------------------------------
// temp_read: invoke returns i16 JSON number
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_temp_read() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "temp_read")
        .expect("temp_read not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(&mut client, entry.cmd_id, &args_schema, &ret_schema, &json!({}))
        .await
        .expect("invoke temp_read");

    assert_eq!(result, json!(100i16));
}

// ---------------------------------------------------------------------------
// rng_u32: invoke returns u32 JSON number
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_rng_u32() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "rng_u32")
        .expect("rng_u32 not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(&mut client, entry.cmd_id, &args_schema, &ret_schema, &json!({}))
        .await
        .expect("invoke rng_u32");

    assert_eq!(result, json!(0xCAFE_BABEu32));
}

// ---------------------------------------------------------------------------
// ficr_uid: invoke returns a positional JSON array [u32, u32] — NOT a named
// object. This contract must match what the MCP host consumer expects.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_ficr_uid_json_is_positional_array() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "ficr_uid")
        .expect("ficr_uid not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(&mut client, entry.cmd_id, &args_schema, &ret_schema, &json!({}))
        .await
        .expect("invoke ficr_uid");

    // A Rust tuple (u32, u32) serializes to a positional array via the MCP bridge.
    // 0xAAAAAAAA = 2863311530, 0x55555555 = 1431655765
    assert_eq!(
        result,
        json!([0xAAAA_AAAAu32, 0x5555_5555u32]),
        "ficr_uid must return a JSON positional array, not a named object"
    );
}

// ---------------------------------------------------------------------------
// saadc_vdd_mv: invoke returns u16 JSON number
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn invoke_saadc_vdd_mv() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    client.discover().expect("discover");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "saadc_vdd_mv")
        .expect("saadc_vdd_mv not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(&mut client, entry.cmd_id, &args_schema, &ret_schema, &json!({}))
        .await
        .expect("invoke saadc_vdd_mv");

    assert_eq!(result, json!(3300u16));
}
