//! Pure conversions between postcard schemas, JSON Schema, and JSON values.
//!
//! Every module here MUST stay side-effect free and synchronous (no I/O, no
//! `async`): the async boundary of `telepath mcp` lives only in the `mcp`
//! module. Keeping the conversions pure is what lets them be covered by plain
//! unit tests.

pub mod json_to_postcard;
pub mod postcard_to_json;
pub mod schema_to_json;
