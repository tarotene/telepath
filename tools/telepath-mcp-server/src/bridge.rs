use crate::json_to_postcard;
use crate::postcard_to_json;
use postcard_schema::schema::owned::OwnedNamedType;
use serde_json::Value;
use telepath_client::{HostError, TelepathClient};

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("args encode error: {0}")]
    ArgsEncode(#[from] json_to_postcard::ConvertError),
    #[error("call_raw error: {0:?}")]
    CallRaw(HostError),
    #[error("response decode error: {0}")]
    ResponseDecode(json_to_postcard::ConvertError),
}

pub async fn invoke<T>(
    client: &mut TelepathClient<T>,
    cmd_id: u16,
    args_schema: &OwnedNamedType,
    ret_schema: &OwnedNamedType,
    args_json: &Value,
) -> Result<Value, BridgeError>
where
    T: std::io::Read + std::io::Write,
{
    let args_bytes = json_to_postcard::json_to_postcard(args_schema, args_json)?;
    let payload = tokio::task::block_in_place(|| client.call_raw(cmd_id, &args_bytes))
        .map_err(BridgeError::CallRaw)?;
    let result = postcard_to_json::postcard_to_json(ret_schema, &payload)
        .map_err(BridgeError::ResponseDecode)?;
    Ok(result)
}
