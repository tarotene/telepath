#![cfg(feature = "mcp")]

mod helpers;

use helpers::{make_pair, spawn_fw};
use rmcp::model::PromptMessageRole;
use rmcp::ServerHandler;
use telepath::mcp::server::{firmware_commands_resource, render_prompt, static_prompts};
use telepath::TelepathMcpServer;
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

// ---------------------------------------------------------------------------
// get_info: capabilities include tools + resources + prompts
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_info_advertises_resources_and_prompts() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let client = TelepathClient::new(host_side);
    let server = TelepathMcpServer::build(client).expect("build server");

    let info = ServerHandler::get_info(&server);
    let caps = info.capabilities;

    assert!(caps.tools.is_some(), "tools capability must be present");
    assert!(
        caps.resources.is_some(),
        "resources capability must be present"
    );
    assert!(caps.prompts.is_some(), "prompts capability must be present");
}

// ---------------------------------------------------------------------------
// firmware_commands_resource: static resource descriptor
// ---------------------------------------------------------------------------

#[test]
fn firmware_commands_resource_has_correct_fields() {
    let r = firmware_commands_resource();
    assert_eq!(r.uri, "telepath://firmware/commands");
    assert_eq!(r.name, "Discovered commands");
    assert_eq!(r.mime_type.as_deref(), Some("application/json"));
    assert!(r.description.is_some());
}

// ---------------------------------------------------------------------------
// commands_catalog_json: serializes tool list to JSON array
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn commands_catalog_json_contains_discovered_commands() {
    let (fw_side, host_side) = make_pair();
    let _guard = spawn_fw(fw_side, run_fw);

    let client = TelepathClient::new(host_side);
    let server = TelepathMcpServer::build(client).expect("build server");
    let json = server.catalog_json_for_test();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON array");
    assert!(
        !parsed.is_empty(),
        "catalog must contain at least one command"
    );

    let names: Vec<&str> = parsed.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"ping"), "ping must appear in catalog");
    assert!(names.contains(&"add"), "add must appear in catalog");

    for entry in &parsed {
        assert!(entry["cmd_id"].is_string(), "cmd_id must be a hex string");
        assert!(
            entry["inputSchema"].is_object(),
            "inputSchema must be an object"
        );
    }
}

// ---------------------------------------------------------------------------
// static_prompts: two prompts, correct names and arguments
// ---------------------------------------------------------------------------

#[test]
fn static_prompts_returns_two_prompts() {
    let prompts = static_prompts();
    assert_eq!(prompts.len(), 2);

    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"verify-board-alive"));
    assert!(names.contains(&"call-command"));
}

#[test]
fn call_command_prompt_has_required_name_arg() {
    let prompts = static_prompts();
    let call_cmd = prompts
        .iter()
        .find(|p| p.name == "call-command")
        .expect("call-command must exist");

    let args = call_cmd
        .arguments
        .as_ref()
        .expect("call-command must have arguments");
    let name_arg = args
        .iter()
        .find(|a| a.name == "name")
        .expect("'name' arg must exist");
    assert_eq!(name_arg.required, Some(true));
}

// ---------------------------------------------------------------------------
// render_prompt: verify-board-alive returns user message
// ---------------------------------------------------------------------------

#[test]
fn render_verify_board_alive_returns_user_message() {
    let result = render_prompt("verify-board-alive", None).expect("prompt must exist");
    assert_eq!(result.messages.len(), 1);
    assert_eq!(result.messages[0].role, PromptMessageRole::User);
}

// ---------------------------------------------------------------------------
// render_prompt: call-command interpolates name and args
// ---------------------------------------------------------------------------

#[test]
fn render_call_command_interpolates_arguments() {
    use serde_json::json;
    let args_obj = json!({"name": "add", "args": "[2, 3]"});
    let args_map = args_obj.as_object().unwrap();

    let result = render_prompt("call-command", Some(args_map)).expect("prompt must exist");
    let text = match &result.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {:?}", other),
    };
    assert!(
        text.contains("add"),
        "command name 'add' must appear in rendered text"
    );
    assert!(text.contains("[2, 3]"), "args must appear in rendered text");
}

#[test]
fn render_call_command_accepts_json_array_args() {
    use serde_json::json;
    let args_obj = json!({"name": "add", "args": [2, 3]});
    let args_map = args_obj.as_object().unwrap();

    let result = render_prompt("call-command", Some(args_map)).expect("prompt must exist");
    let text = match &result.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {:?}", other),
    };
    assert!(
        text.contains("add"),
        "command name 'add' must appear in rendered text"
    );
    assert!(
        text.contains("2") && text.contains("3"),
        "array elements must appear in rendered text; got: {text}"
    );
}

#[test]
fn render_call_command_uses_placeholder_when_no_args() {
    let result = render_prompt("call-command", None).expect("prompt must exist");
    let text = match &result.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {:?}", other),
    };
    assert!(
        text.contains("<command>"),
        "placeholder must appear when no name arg provided"
    );
}

// ---------------------------------------------------------------------------
// render_prompt: unknown name returns None
// ---------------------------------------------------------------------------

#[test]
fn render_unknown_prompt_returns_none() {
    assert!(render_prompt("nonexistent-prompt", None).is_none());
}
