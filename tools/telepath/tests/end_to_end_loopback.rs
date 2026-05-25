mod helpers;

use helpers::{make_pair, spawn_fw};
use serde_json::json;
use telepath::bridge;
use telepath_client::TelepathClient;
use telepath_server::{command, TelepathServer};

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn run_fw(fw_side: helpers::FwSide, running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let mut server = TelepathServer::<_, 512>::new(fw_side, telepath_server::commands());
    while running.load(std::sync::atomic::Ordering::Acquire) {
        server.poll();
        std::thread::yield_now();
    }
}

#[test]
fn discover_and_invoke_ping() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let mut client = TelepathClient::new(host_side);
    let n = client.discover().expect("discover");
    assert!(n >= 2, "expected at least 2 commands, got {n}");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "ping")
        .expect("ping not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result = bridge::invoke(
        &mut client,
        entry.cmd_id,
        &args_schema,
        &ret_schema,
        &json!({}),
    )
    .expect("invoke ping");

    assert_eq!(result, json!(0xDEAD_BEEFu32));
}

#[test]
fn discover_and_invoke_add() {
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
    .expect("invoke add");

    assert_eq!(result, json!(5));
}
