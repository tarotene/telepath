//! Telepath wire protocol types.
//!
//! This crate is `no_std` and `no alloc`. It defines the shared types used by
//! both the firmware-side (`telepath-firmware`) and host-side (`telepath-host`)
//! libraries. All types must remain free of heap allocation.
//!
//! # Protocol overview
//!
//! - Framing: COBS both directions in current MVP; rzCOBS upstream planned for Stage C2
//! - Serialization: postcard (little-endian, varint-compressed)
//! - Packet type discriminant: 2-valued (Request / Response), per ONC RPC CALL/REPLY model
//! - Errors: expressed via `ResponseStatus`, not as a separate packet type
//! - Discovery: reserved CmdID 0x0000 (CoAP Empty / ONC RPC convention)
#![no_std]

pub mod cmd_id;
pub mod framing;

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
// WireError
// ---------------------------------------------------------------------------

/// Errors that can arise during wire-level encoding or decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// Payload exceeded [`MAX_PAYLOAD_SIZE`].
    PayloadTooLarge,
    /// postcard serialization / deserialization failed.
    SerdeError,
    /// A reserved or unknown packet type discriminant was received.
    UnknownPacketType,
    /// The framing delimiter was encountered in an unexpected position.
    FramingError,
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
}
