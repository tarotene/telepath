//! Target-side Telepath library.
//!
//! Runs on the MCU in `no_std` mode. Provides:
//! - [`TelepathServer`]: receive loop, COBS decode, dispatch, rzCOBS encode
//! - Re-export of `#[command]` attribute macro
//!
//! # Architecture
//!
//! ```text
//! Transport (embedded-io) -> CobsDecoder -> Dispatcher -> CommandRegistry
//!                                                       -> rzCOBS encode -> Transport
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use telepath_firmware::{TelepathServer, command};
//!
//! #[command]
//! fn set_led(id: u8, brightness: u16) -> Result<(), ()> {
//!     Ok(())
//! }
//!
//! // In your embassy main:
//! let mut server = TelepathServer::<_, 256>::new(transport);
//! loop { server.poll(); }
//! ```
#![no_std]

pub use telepath_macros::command;
pub use telepath_wire::{
    PacketType, ResponseStatus, WireError, CMD_ID_DISCOVERY, MAX_PAYLOAD_SIZE,
};

// ---------------------------------------------------------------------------
// CommandMetadata
// ---------------------------------------------------------------------------

/// Type-erased shim function signature.
///
/// Receives a postcard-serialized argument slice, writes a postcard-serialized
/// result into `output`, and returns the number of bytes written.
pub type ShimFn = fn(input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError>;

/// Static metadata for a single registered RPC command.
///
/// Populated by the `#[command]` macro at compile time and collected into a
/// contiguous array via `linkme` distributed slices (planned).
pub struct CommandMetadata {
    /// Human-readable function name (used for discovery).
    pub name: &'static str,
    /// Command ID: hash of (name + input schema + output schema).
    /// Computed at firmware build time for deterministic matching.
    pub id: u16,
    /// Type-erased shim that deserializes args, calls the function, and
    /// serializes the result.
    pub invoke: ShimFn,
}

// ---------------------------------------------------------------------------
// DispatchError
// ---------------------------------------------------------------------------

/// Errors that can occur during command dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    /// No command with the given ID was found in the registry.
    UnknownCommand,
    /// Argument deserialization failed (malformed or truncated payload).
    DeserializeError,
    /// Result serialization failed (output buffer too small).
    SerializeError,
    /// The request payload exceeded [`MAX_PAYLOAD_SIZE`].
    PayloadTooLarge,
}

// ---------------------------------------------------------------------------
// TelepathServer
// ---------------------------------------------------------------------------

/// RPC server that runs on the target MCU.
///
/// `T` is the transport type implementing byte-stream I/O.
/// `N` is the size of the internal receive and transmit buffers (bytes).
///
/// # Type parameter guidance
///
/// Choose `N` to be at least `MAX_PAYLOAD_SIZE` plus framing overhead.
/// A value of 512 is recommended for most use cases.
#[allow(dead_code)] // fields wired up once framing layer is implemented
pub struct TelepathServer<T, const N: usize> {
    transport: T,
    rx_buf: [u8; N],
    tx_buf: [u8; N],
    /// Command registry slice. Populated via `linkme` once the macro is complete.
    /// For now, users may pass a static slice manually.
    commands: &'static [CommandMetadata],
}

impl<T, const N: usize> TelepathServer<T, N> {
    /// Create a new server with the given transport and command registry.
    pub fn new(transport: T, commands: &'static [CommandMetadata]) -> Self {
        Self {
            transport,
            rx_buf: [0u8; N],
            tx_buf: [0u8; N],
            commands,
        }
    }

    /// Look up a command by its ID using linear scan.
    ///
    /// Linear scan is intentional: embedded command counts are typically ≤ 64,
    /// making hash-map overhead unjustified. At 64 entries and 64 MHz this
    /// resolves in < 1 µs.
    pub fn find_command(&self, id: u16) -> Option<&CommandMetadata> {
        self.commands.iter().find(|cmd| cmd.id == id)
    }

    /// Dispatch a pre-decoded payload slice to the matching command handler.
    ///
    /// Returns the number of bytes written to `output` on success.
    pub fn dispatch(
        &mut self,
        cmd_id: u16,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<usize, DispatchError> {
        if cmd_id == telepath_wire::CMD_ID_DISCOVERY {
            return self.handle_discovery(output);
        }
        let cmd = self
            .find_command(cmd_id)
            .ok_or(DispatchError::UnknownCommand)?;
        (cmd.invoke)(input, output)
    }

    /// Handle a Discovery request (CmdID 0x0000).
    ///
    /// Writes a minimal discovery response listing all registered command names
    /// and IDs. Full schema transport is deferred to the Schema-on-Demand flow.
    fn handle_discovery(&self, _output: &mut [u8]) -> Result<usize, DispatchError> {
        // TODO: serialize CommandMetadata list via postcard into output buffer.
        // Returns 0 bytes until CDP serialization is implemented.
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_shim(_input: &[u8], _output: &mut [u8]) -> Result<usize, DispatchError> {
        Ok(0)
    }

    static TEST_COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
        name: "ping",
        id: 0x0001,
        invoke: noop_shim,
    }];

    struct FakeTransport;

    #[test]
    fn find_known_command() {
        let server = TelepathServer::<FakeTransport, 256>::new(FakeTransport, &TEST_COMMANDS);
        assert!(server.find_command(0x0001).is_some());
    }

    #[test]
    fn find_unknown_command_returns_none() {
        let server = TelepathServer::<FakeTransport, 256>::new(FakeTransport, &TEST_COMMANDS);
        assert!(server.find_command(0xFFFF).is_none());
    }

    #[test]
    fn dispatch_unknown_returns_error() {
        let mut server = TelepathServer::<FakeTransport, 256>::new(FakeTransport, &TEST_COMMANDS);
        let mut out = [0u8; 256];
        assert_eq!(
            server.dispatch(0xFFFF, &[], &mut out),
            Err(DispatchError::UnknownCommand)
        );
    }
}
