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
use telepath_client::{HostError, TelepathClient};
use tokio::sync::Mutex;

struct ToolMeta {
    tool: Tool,
    cmd_id: u16,
    args_schema: OwnedNamedType,
    ret_schema: OwnedNamedType,
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
        let entries: Vec<_> = client.schema_cache().iter().cloned().collect();
        let mut tools = Vec::new();
        for entry in entries {
            let args_schema = entry.decoded_args_schema()?;
            let ret_schema = entry.decoded_ret_schema()?;
            let raw_schema = named_type_to_json_schema(&args_schema);
            // MCP spec requires inputSchema.type to be "object".
            // Unit commands → {"type":"object"} (no properties).
            // Tuple commands → {"type":"object","properties":{"args":<array_schema>},"required":["args"]}
            // so that call_tool can extract args["args"] as the positional array.
            let input_schema_json =
                if raw_schema.get("type").and_then(|v| v.as_str()) == Some("null") {
                    serde_json::json!({"type": "object"})
                } else {
                    serde_json::json!({
                        "type": "object",
                        "properties": { "args": raw_schema },
                        "required": ["args"]
                    })
                };
            let input_schema = rmcp::model::object(input_schema_json);
            let description = format!("Telepath command 0x{:04X}", entry.cmd_id);
            let tool = Tool::new(entry.name.clone(), description, input_schema);
            tools.push(ToolMeta {
                tool,
                cmd_id: entry.cmd_id,
                args_schema,
                ret_schema,
            });
        }
        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            tools,
        })
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

        let args_json: Value = request
            .arguments
            .and_then(|obj| obj.get("args").cloned())
            .unwrap_or(Value::Null);

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

fn bridge_to_mcp_error(e: BridgeError) -> ErrorData {
    let code = match &e {
        BridgeError::ArgsEncode(_) => rmcp::model::ErrorCode::INVALID_PARAMS,
        BridgeError::CallRaw(_) | BridgeError::ResponseDecode(_) => {
            rmcp::model::ErrorCode::INTERNAL_ERROR
        }
    };
    ErrorData::new(code, e.to_string(), None)
}
