use crate::bridge::{self, BridgeError};
use crate::schema_to_json::named_type_to_json_schema;
use postcard_schema::schema::owned::OwnedNamedType;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
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
    /// Argument names in declaration order (empty for zero-arg commands).
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

    /// Re-run the Command Discovery Protocol and rebuild the tool list.
    ///
    /// Call this after a firmware reflash or transport reconnect to pick up
    /// new or changed `#[command]` registrations. Automatically triggers
    /// [`TelepathClient::rediscover`] on the underlying client.
    pub async fn rediscover(&mut self) -> Result<(), HostError> {
        let entries = {
            let mut client = self.client.lock().await;
            client.rediscover()?;
            client.schema_cache().iter().cloned().collect::<Vec<_>>()
        };
        self.tools = build_tools(entries)?;
        Ok(())
    }
}

impl<T> ServerHandler for TelepathMcpServer<T>
where
    T: std::io::Read + std::io::Write + Send + 'static,
{
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
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
        let result = bridge::invoke(
            &mut *client,
            meta.cmd_id,
            &meta.args_schema,
            &meta.ret_schema,
            &args_json,
        )
        .await
        .map_err(bridge_to_mcp_error)?;

        // structured() sets structuredContent to a raw Value, which fails MCP
        // spec Zod validation when the value is not an object.  Return the
        // JSON representation as plain text instead.
        Ok(CallToolResult::success(vec![Content::text(
            result.to_string(),
        )]))
    }
}

fn build_tools(entries: Vec<SchemaEntry>) -> Result<Vec<ToolMeta>, HostError> {
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

/// Build an MCP-compliant `inputSchema` (`"type": "object"`) from the raw tuple schema
/// and the declared argument names.
///
/// - 0 args  → `{"type": "object"}`
/// - N args  → `{"type":"object","properties":{"<name>": <elem_schema>, ...},"required":[...]}`
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

/// Convert a named-argument MCP object (`{"a": 2, "b": 3}`) into the positional
/// JSON array expected by `json_to_postcard` (`[2, 3]`).
///
/// Returns `Value::Null` for zero-argument commands (empty `arg_names`).
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
