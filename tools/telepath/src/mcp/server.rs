use crate::bridge::{self, BridgeError};
use crate::codec::schema_to_json::named_type_to_json_schema;
use postcard_schema::schema::owned::OwnedNamedType;
use rmcp::model::{
    AnnotateAble, CallToolRequestParams, CallToolResult, Content, GetPromptRequestParams,
    GetPromptResult, InitializeResult, ListPromptsResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, PromptMessageRole, RawResource,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};
use serde_json::Value;
use std::sync::Arc;
use telepath_client::{HostError, SchemaEntry, TelepathClient};
use tokio::sync::Mutex;

struct ToolMeta {
    tool: Tool,
    cmd_id: u16,
    args_schema: OwnedNamedType,
    ret_schema: OwnedNamedType,
    arg_names: Vec<String>,
}

pub struct TelepathMcpServer<T>
where
    T: std::io::Read + std::io::Write + Send + 'static,
{
    client: Arc<Mutex<TelepathClient<T>>>,
    tools: Vec<ToolMeta>,
}

impl<T> TelepathMcpServer<T>
where
    T: std::io::Read + std::io::Write + Send + 'static,
{
    pub fn build(mut client: TelepathClient<T>) -> Result<Self, HostError> {
        client.discover()?;
        let tools = build_tools(client.schema_cache().iter().cloned().collect())?;
        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            tools,
        })
    }

    pub async fn rediscover(&mut self) -> Result<(), HostError> {
        let entries = {
            let mut client = self.client.lock().await;
            tokio::task::block_in_place(|| client.rediscover())?;
            client.schema_cache().iter().cloned().collect::<Vec<_>>()
        };
        match build_tools(entries) {
            Ok(tools) => {
                self.tools = tools;
                Ok(())
            }
            Err(e) => {
                self.tools.clear();
                Err(e)
            }
        }
    }

    #[doc(hidden)]
    pub fn catalog_json_for_test(&self) -> String {
        commands_catalog_json(&self.tools)
    }
}

impl<T> ServerHandler for TelepathMcpServer<T>
where
    T: std::io::Read + std::io::Write + Send + 'static,
{
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_instructions("Exposes Telepath #[command] functions as MCP tools.")
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let list = self.tools.iter().map(|m| m.tool.clone()).collect();
        Ok(ListToolsResult::with_all_items(list))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            firmware_commands_resource(),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        if request.uri != "telepath://firmware/commands" {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown resource: {}", request.uri),
                None,
            ));
        }
        let json = commands_catalog_json(&self.tools);
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: request.uri,
                mime_type: Some("application/json".into()),
                text: json,
                meta: None,
            },
        ]))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(static_prompts()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, ErrorData> {
        render_prompt(&request.name, request.arguments.as_ref()).ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown prompt: {}", request.name),
                None,
            )
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let meta = self
            .tools
            .iter()
            .find(|m| m.tool.name == request.name)
            .ok_or_else(|| {
                ErrorData::new(
                    rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                    format!("unknown tool: {}", request.name),
                    None,
                )
            })?;

        let args_json: Value = named_to_positional(request.arguments, &meta.arg_names);

        let mut client = self.client.lock().await;
        let result = tokio::task::block_in_place(|| {
            bridge::invoke(
                &mut *client,
                meta.cmd_id,
                &meta.args_schema,
                &meta.ret_schema,
                &args_json,
            )
        })
        .map_err(bridge_to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            result.to_string(),
        )]))
    }
}

fn build_tools(mut entries: Vec<SchemaEntry>) -> Result<Vec<ToolMeta>, HostError> {
    // Sort by cmd_id for deterministic tool ordering within a run.
    entries.sort_by_key(|e| e.cmd_id);

    // Detect name collisions: two commands with the same name but different
    // cmd_ids (i.e. different signatures) would appear as duplicate MCP tool
    // names, which clients cannot disambiguate. Warn on stderr so the operator
    // is aware.
    {
        let mut names: std::collections::HashMap<&str, Vec<u16>> =
            std::collections::HashMap::new();
        for e in &entries {
            names.entry(&e.name).or_default().push(e.cmd_id);
        }
        for (name, ids) in &names {
            if ids.len() > 1 {
                eprintln!(
                    "[telepath mcp] Warning: ambiguous command name '{name}' \
                     maps to {} cmd_ids ({:04X?}). MCP clients cannot \
                     disambiguate; only the first will be reachable by name.",
                    ids.len(),
                    ids
                );
            }
        }
    }

    let mut tools = Vec::new();
    for entry in entries {
        let args_schema = entry.decoded_args_schema()?;
        let ret_schema = entry.decoded_ret_schema()?;
        let input_schema_json =
            build_input_schema(&named_type_to_json_schema(&args_schema), &entry.arg_names);
        let input_schema = rmcp::model::object(input_schema_json);
        let description = format!("Telepath command 0x{:04X}", entry.cmd_id);
        let tool = Tool::new(entry.name.clone(), description, input_schema);
        tools.push(ToolMeta {
            tool,
            cmd_id: entry.cmd_id,
            args_schema,
            ret_schema,
            arg_names: entry.arg_names.clone(),
        });
    }
    Ok(tools)
}

fn bridge_to_mcp_error(e: BridgeError) -> ErrorData {
    let code = match &e {
        BridgeError::ArgsEncode(_) => rmcp::model::ErrorCode::INVALID_PARAMS,
        BridgeError::CallRaw(_) | BridgeError::ResponseDecode(_) => {
            rmcp::model::ErrorCode::INTERNAL_ERROR
        }
    };
    ErrorData::new(code, e.to_string(), None)
}

fn build_input_schema(raw_schema: &Value, arg_names: &[String]) -> Value {
    if arg_names.is_empty() {
        return serde_json::json!({"type": "object"});
    }
    let prefix_items: Vec<Value> = raw_schema
        .get("prefixItems")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, schema) in arg_names.iter().zip(prefix_items.iter()) {
        properties.insert(name.clone(), schema.clone());
        required.push(Value::String(name.clone()));
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn named_to_positional(
    arguments: Option<serde_json::Map<String, Value>>,
    arg_names: &[String],
) -> Value {
    if arg_names.is_empty() {
        return Value::Null;
    }
    let obj = arguments.unwrap_or_default();
    let arr: Vec<Value> = arg_names
        .iter()
        .map(|name| obj.get(name).cloned().unwrap_or(Value::Null))
        .collect();
    Value::Array(arr)
}

pub fn firmware_commands_resource() -> rmcp::model::Resource {
    RawResource::new("telepath://firmware/commands", "Discovered commands")
        .with_description("All #[command] functions discovered from the connected firmware via CDP")
        .with_mime_type("application/json")
        .no_annotation()
}

fn commands_catalog_json(tools: &[ToolMeta]) -> String {
    let catalog: Vec<Value> = tools
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.tool.name,
                "cmd_id": format!("0x{:04X}", m.cmd_id),
                "inputSchema": *m.tool.input_schema,
            })
        })
        .collect();
    serde_json::to_string_pretty(&catalog).unwrap_or_else(|_| "[]".to_string())
}

pub fn static_prompts() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "verify-board-alive",
            Some("Check that the connected firmware is responsive"),
            None::<Vec<PromptArgument>>,
        ),
        Prompt::new(
            "call-command",
            Some("Call a named Telepath command with optional arguments"),
            Some(vec![
                PromptArgument::new("name")
                    .with_description("Name of the command to call")
                    .with_required(true),
                PromptArgument::new("args")
                    .with_description("JSON arguments for the command (omit for zero-arg commands)")
                    .with_required(false),
            ]),
        ),
    ]
}

pub fn render_prompt(
    name: &str,
    arguments: Option<&rmcp::model::JsonObject>,
) -> Option<GetPromptResult> {
    match name {
        "verify-board-alive" => Some(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            "Use the 'ping' tool to verify the firmware is alive. \
             The ping command takes no arguments and returns a 32-bit sentinel value. \
             Report success or failure and the value you received.",
        )])),
        "call-command" => {
            let cmd = arguments
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("<command>");
            let args = match arguments.and_then(|a| a.get("args")) {
                None => "{}".to_string(),
                Some(Value::String(s)) => s.clone(),
                Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
            };
            Some(GetPromptResult::new(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "Call the Telepath command '{cmd}' with the following arguments: {args}. \
                     Report the result value or any error that occurs.",
                ),
            )]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_input_schema_zero_args_returns_empty_object() {
        let raw = json!({"type": "null"});
        let schema = build_input_schema(&raw, &[]);
        assert_eq!(schema, json!({"type": "object"}));
    }

    #[test]
    fn build_input_schema_two_args_maps_names_to_element_schemas() {
        let raw = json!({
            "type": "array",
            "prefixItems": [
                {"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64},
                {"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64}
            ],
            "minItems": 2u64,
            "maxItems": 2u64
        });
        let names = vec!["a".to_string(), "b".to_string()];
        let schema = build_input_schema(&raw, &names);
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["a"].is_object());
        assert!(schema["properties"]["b"].is_object());
        assert_eq!(schema["required"], json!(["a", "b"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn named_to_positional_zero_args_returns_null() {
        let result = named_to_positional(None, &[]);
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn named_to_positional_two_args_preserves_declaration_order() {
        let names = vec!["a".to_string(), "b".to_string()];
        let mut obj = serde_json::Map::new();
        obj.insert("b".to_string(), json!(3));
        obj.insert("a".to_string(), json!(2));
        let result = named_to_positional(Some(obj), &names);
        assert_eq!(result, json!([2, 3]));
    }

    #[test]
    fn named_to_positional_missing_arg_becomes_null() {
        let names = vec!["x".to_string()];
        let result = named_to_positional(None, &names);
        assert_eq!(result, json!([null]));
    }
}
