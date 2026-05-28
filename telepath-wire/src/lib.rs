//! Telepath wire protocol types.
//!
//! This crate is `no_std` and `no alloc`. It defines the shared types used by
//! both the firmware-side (`telepath-firmware`) and host-side (`telepath-host`)
//! libraries. All types must remain free of heap allocation.
//!
//! # Protocol overview
//!
//! - Framing: COBS downstream (Host→Target), rzCOBS upstream (Target→Host); 0x00 delimiter both directions
//! - Serialization: postcard (little-endian, varint-compressed)
//! - Packet type discriminant: 2-valued (Request / Response), per ONC RPC CALL/REPLY model
//! - Errors: expressed via `ResponseStatus`, not as a separate packet type
//! - Discovery: reserved CmdID 0x0000 (CoAP Empty / ONC RPC convention)
#![no_std]

pub mod cmd_id;
pub mod framing;
#[cfg(feature = "profile")]
pub mod metrics;
#[cfg(feature = "profile")]
pub use metrics::MetricsSnapshot;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum payload size in bytes. Both sides MUST enforce this limit.
pub const MAX_PAYLOAD_SIZE: usize = 256;

/// Reserved command ID for the Command Discovery Protocol (CDP).
///
/// Sending a `Request` with this ID causes the target to reply with its full
/// command registry. Modeled after RFC 7252 CoAP Code 0.00 (Empty) and ONC RPC
/// NULL procedure convention.
pub const CMD_ID_DISCOVERY: u16 = 0x0000;

/// Reserved command ID for the framing/throughput metrics snapshot.
///
/// Sending a `Request` with this ID causes the target to reply with the current
/// [`MetricsSnapshot`] and atomically reset all counters. Only present when the
/// target is built with the `profile` feature; without it the command simply
/// does not exist and the host receives `SystemError`.
pub const CMD_ID_METRICS: u16 = 0xFFFE;

// ---------------------------------------------------------------------------
// PacketType
// ---------------------------------------------------------------------------

/// Wire-level packet type discriminant.
///
/// Only two variants exist (Request / Response), following the ONC RPC
/// RFC 5531 CALL/REPLY model. Error information is carried inside a
/// `Response` via [`ResponseStatus`], not as a distinct packet type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketType {
    /// A call from host to target (CALL in ONC RPC terminology).
    Request = 0x01,
    /// A reply from target to host (REPLY in ONC RPC terminology).
    Response = 0x02,
}

// ---------------------------------------------------------------------------
// ResponseStatus
// ---------------------------------------------------------------------------

/// Status code carried inside a [`Response`] packet.
///
/// Using a dedicated status field (rather than a separate packet type) keeps
/// the framing layer simple and mirrors HTTP status code semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResponseStatus {
    /// The command executed successfully.
    Ok = 0x00,
    /// The command returned a user-defined application error.
    AppError = 0x01,
    /// A system-level error occurred (e.g., unknown CmdID, deserialize failure).
    SystemError = 0x02,
}

// ---------------------------------------------------------------------------
// Request / Response packets
// ---------------------------------------------------------------------------

/// An RPC call from host to target.
///
/// The `args` field is a postcard-serialized argument struct. Its schema is
/// identified by `cmd_id`, which encodes the function name and argument types
/// as a hash (computed at firmware build time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request<'a> {
    /// Packet kind — always [`PacketType::Request`] on the wire.
    pub kind: PacketType,
    /// Monotonically increasing sequence number for matching responses to calls.
    pub seq_no: u16,
    /// Command identifier: hash of (function name + input schema + output schema).
    pub cmd_id: u16,
    /// Postcard-serialized argument bytes. Lifetime tied to the receive buffer.
    #[serde(borrow)]
    pub args: &'a [u8],
}

/// An RPC reply from target to host.
///
/// On success (`status == Ok`) `payload` contains the postcard-serialized
/// return value. On error it contains a postcard-serialized error description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response<'a> {
    /// Packet kind — always [`PacketType::Response`] on the wire.
    pub kind: PacketType,
    /// Matches the `seq_no` of the originating [`Request`].
    pub seq_no: u16,
    /// Execution outcome.
    pub status: ResponseStatus,
    /// Postcard-serialized return value or error payload.
    #[serde(borrow)]
    pub payload: &'a [u8],
}

// ---------------------------------------------------------------------------
// DiscoveryEntry
// ---------------------------------------------------------------------------

/// A single entry returned by the Command Discovery Protocol (CmdID 0x0000).
///
/// The firmware serializes all registered commands as a postcard sequence of
/// these entries in response to a CDP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEntry<'a> {
    /// 16-bit command ID derived by [`telepath_wire::cmd_id::derive_cmd_id`].
    pub id: u16,
    /// Rust function name of the registered command.
    #[serde(borrow)]
    pub name: &'a str,
    /// Postcard-serialized `postcard_schema::schema::NamedType` for the
    /// argument tuple. Opaque bytes; decode with `postcard_schema` on the host.
    #[serde(borrow)]
    pub args_schema: &'a [u8],
    /// Postcard-serialized `postcard_schema::schema::NamedType` for the
    /// return type.
    #[serde(borrow)]
    pub ret_schema: &'a [u8],
    /// Comma-separated argument names, e.g. `"a,b"` for `fn foo(a: i32, b: i32)`.
    /// Empty string for zero-argument commands.
    #[serde(borrow)]
    pub arg_names: &'a str,
}

// ---------------------------------------------------------------------------
// Discovery paging types (CmdID 0x0000 with offset-based pagination)
// ---------------------------------------------------------------------------

/// Request payload for the Command Discovery Protocol when pagination is needed.
///
/// Empty `args` (legacy) is treated as `DiscoveryRequest { offset: 0 }` by
/// the firmware for backward compatibility. Non-empty args must deserialize
/// to this type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DiscoveryRequest {
    /// Index of the first entry to include in this response page.
    pub offset: u16,
}

/// Response payload for a paged Command Discovery Protocol response.
///
/// `entries` carries a raw postcard sequence: `varint(count) ++ DiscoveryEntry × count`.
/// The host iterates pages until `offset + count_this_page >= total`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPage<'a> {
    /// Total number of registered commands (across all pages).
    pub total: u16,
    /// Offset this page starts at (echoes the request's `offset` field).
    pub offset: u16,
    /// Serialized `varint(count) ++ DiscoveryEntry × count` for entries in
    /// this page. Opaque to this crate; parse with `postcard::take_from_bytes`.
    #[serde(borrow)]
    pub entries: &'a [u8],
}

// ---------------------------------------------------------------------------
// AppError payload
// ---------------------------------------------------------------------------

/// Payload carried inside a [`Response`] when `status == ResponseStatus::AppError`.
///
/// Encoded as postcard `(varint(code), varint(len), len-bytes UTF-8 message)`.
/// Borrows the message slice from the receive buffer for zero-copy decode on
/// targets that cannot allocate.
///
/// # Wire layout
///
/// | Field | Type | Encoding |
/// |-------|------|----------|
/// | `code` | `u16` | postcard varint (1–3 bytes) |
/// | `message` | `&str` | postcard varint(len) + UTF-8 bytes |
///
/// The `code` namespace is application-defined. Reserve `0` as a catch-all
/// "unspecified application error" when no finer classification is needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppErrorPayload<'a> {
    /// Application-defined error code.
    pub code: u16,
    /// Human-readable error message, borrowed from the receive buffer.
    #[serde(borrow)]
    pub message: &'a str,
}

/// Encode an [`AppErrorPayload`] into `out`, returning the number of bytes written.
///
/// # Errors
///
/// Returns [`WireError::SerdeError`] if the payload does not fit in `out`.
pub fn encode_app_error(payload: &AppErrorPayload<'_>, out: &mut [u8]) -> Result<usize, WireError> {
    let written = postcard::to_slice(payload, out)?;
    Ok(written.len())
}

/// Decode an [`AppErrorPayload`] from `bytes`, borrowing the message slice.
///
/// # Errors
///
/// Returns [`WireError::SerdeError`] if `bytes` is malformed.
pub fn decode_app_error(bytes: &[u8]) -> Result<AppErrorPayload<'_>, WireError> {
    Ok(postcard::from_bytes(bytes)?)
}

// ---------------------------------------------------------------------------
// WireError
// ---------------------------------------------------------------------------

/// Errors that can arise during wire-level encoding or decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// Payload exceeded [`MAX_PAYLOAD_SIZE`].
    PayloadTooLarge,
    /// postcard serialization / deserialization failed; carries the underlying cause.
    SerdeError(postcard::Error),
    /// A reserved or unknown packet type discriminant was received.
    UnknownPacketType,
    /// The framing delimiter was encountered in an unexpected position.
    FramingError,
}

impl From<postcard::Error> for WireError {
    fn from(e: postcard::Error) -> Self {
        WireError::SerdeError(e)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_type_discriminants() {
        assert_eq!(PacketType::Request as u8, 0x01);
        assert_eq!(PacketType::Response as u8, 0x02);
    }

    #[test]
    fn response_status_discriminants() {
        assert_eq!(ResponseStatus::Ok as u8, 0x00);
        assert_eq!(ResponseStatus::AppError as u8, 0x01);
        assert_eq!(ResponseStatus::SystemError as u8, 0x02);
    }

    #[test]
    fn cmd_id_discovery_is_zero() {
        assert_eq!(CMD_ID_DISCOVERY, 0x0000);
    }

    #[test]
    fn max_payload_size() {
        assert_eq!(MAX_PAYLOAD_SIZE, 256);
    }

    #[test]
    fn wire_error_from_postcard_error() {
        let pe = postcard::from_bytes::<u32>(&[]).unwrap_err();
        let we: WireError = pe.clone().into();
        assert_eq!(we, WireError::SerdeError(pe));
    }

    #[test]
    fn app_error_payload_roundtrip() {
        let original = AppErrorPayload {
            code: 42,
            message: "sensor not ready",
        };
        let mut buf = [0u8; 64];
        let n = encode_app_error(&original, &mut buf).expect("encode failed");
        let decoded = decode_app_error(&buf[..n]).expect("decode failed");
        assert_eq!(decoded.code, original.code);
        assert_eq!(decoded.message, original.message);
    }

    #[test]
    fn app_error_payload_wire_layout() {
        // code=42 (0x2A, 1 varint byte), message="hi" (len=2, bytes 0x68 0x69)
        let payload = AppErrorPayload {
            code: 42,
            message: "hi",
        };
        let mut buf = [0u8; 16];
        let n = encode_app_error(&payload, &mut buf).expect("encode failed");
        assert_eq!(&buf[..n], &[0x2A, 0x02, b'h', b'i']);
    }

    #[test]
    fn app_error_payload_buffer_too_small() {
        let payload = AppErrorPayload {
            code: 42,
            message: "hi",
        };
        let mut buf = [0u8; 2]; // too small (needs 4 bytes)
        let err = encode_app_error(&payload, &mut buf).unwrap_err();
        assert!(matches!(err, WireError::SerdeError(_)));
    }
}
