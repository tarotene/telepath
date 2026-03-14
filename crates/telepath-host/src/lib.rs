//! Host-side Telepath library.
//!
//! Provides [`TelepathClient`] for issuing RPC calls to a target device, and
//! [`SchemaCache`] for caching discovered command schemas keyed by `cmd_id`.
//!
//! # Usage
//!
//! ```rust,ignore
//! use telepath_host::{TelepathClient, SchemaCache};
//!
//! let mut client = TelepathClient::new(serial_port);
//! let schemas = client.discover().unwrap();
//! let result = client.call_raw(0x0001, &args_bytes).unwrap();
//! ```

use telepath_wire::{CMD_ID_DISCOVERY, MAX_PAYLOAD_SIZE};

// ---------------------------------------------------------------------------
// HostError
// ---------------------------------------------------------------------------

/// Errors that can arise on the host side.
#[derive(Debug)]
pub enum HostError {
    /// I/O error from the underlying transport.
    Io(String),
    /// Response sequence number did not match the pending request.
    SeqMismatch { expected: u16, got: u16 },
    /// The target reported a system-level error.
    SystemError,
    /// The target reported an application-level error.
    AppError(Vec<u8>),
    /// postcard serialization or deserialization failed.
    SerdeError,
    /// The response payload exceeded [`MAX_PAYLOAD_SIZE`].
    PayloadTooLarge,
}

// ---------------------------------------------------------------------------
// SchemaEntry
// ---------------------------------------------------------------------------

/// A cached command schema entry retrieved from the target via CDP.
#[derive(Debug, Clone)]
pub struct SchemaEntry {
    /// Command name as reported by the target firmware.
    pub name: String,
    /// Command ID: hash of (name + input schema + output schema).
    pub cmd_id: u16,
    /// Raw postcard-encoded argument schema bytes (opaque until schema parsing
    /// is implemented).
    pub args_schema: Vec<u8>,
    /// Raw postcard-encoded return schema bytes.
    pub ret_schema: Vec<u8>,
}

// ---------------------------------------------------------------------------
// SchemaCache
// ---------------------------------------------------------------------------

/// In-memory cache of command schemas keyed by `cmd_id`.
///
/// Cache coherence is guaranteed by the `cmd_id` design: the ID is a hash of
/// (name + input schema + output schema), so any firmware change that alters
/// an argument type automatically produces a new ID, triggering a cache miss.
#[derive(Debug, Default)]
pub struct SchemaCache {
    entries: std::collections::HashMap<u16, SchemaEntry>,
}

impl SchemaCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a schema entry.
    pub fn insert(&mut self, entry: SchemaEntry) {
        self.entries.insert(entry.cmd_id, entry);
    }

    /// Look up a schema entry by command ID.
    pub fn get(&self, cmd_id: u16) -> Option<&SchemaEntry> {
        self.entries.get(&cmd_id)
    }

    /// Remove a stale entry (e.g. after a firmware update changes the ID).
    pub fn invalidate(&mut self, cmd_id: u16) {
        self.entries.remove(&cmd_id);
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// TelepathClient
// ---------------------------------------------------------------------------

/// RPC client that communicates with a [`telepath_firmware::TelepathServer`].
///
/// `T` must implement `std::io::Read + std::io::Write` (e.g. `serialport`).
#[allow(dead_code)] // transport wired up once framing layer is implemented
pub struct TelepathClient<T> {
    transport: T,
    schema_cache: SchemaCache,
    seq_counter: u16,
}

impl<T: std::io::Read + std::io::Write> TelepathClient<T> {
    /// Create a new client wrapping the given transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            schema_cache: SchemaCache::new(),
            seq_counter: 0,
        }
    }

    /// Run the Command Discovery Protocol.
    ///
    /// Sends a [`CMD_ID_DISCOVERY`] request and populates the schema cache
    /// with the returned command list. Returns the number of commands found.
    pub fn discover(&mut self) -> Result<usize, HostError> {
        // TODO: serialize Discovery Request, send via COBS-framed transport,
        // receive rzCOBS-framed response, populate schema_cache.
        // Returns 0 until framing/transport layer is implemented.
        let _ = CMD_ID_DISCOVERY;
        Ok(0)
    }

    /// Issue a raw RPC call.
    ///
    /// `cmd_id` identifies the target command; `args` is a postcard-serialized
    /// argument struct. Returns the postcard-serialized response payload on
    /// success.
    ///
    /// The caller is responsible for serializing args and deserializing the
    /// response. Higher-level typed wrappers will be added in future releases.
    pub fn call_raw(&mut self, cmd_id: u16, args: &[u8]) -> Result<Vec<u8>, HostError> {
        if args.len() > MAX_PAYLOAD_SIZE {
            return Err(HostError::PayloadTooLarge);
        }
        let seq = self.next_seq();
        // TODO: frame Request { kind, seq, cmd_id, args } via COBS and send.
        // TODO: receive rzCOBS-framed Response, validate seq_no and status.
        let _ = (seq, cmd_id);
        Ok(Vec::new())
    }

    /// Borrow the schema cache for inspection.
    pub fn schema_cache(&self) -> &SchemaCache {
        &self.schema_cache
    }

    fn next_seq(&mut self) -> u16 {
        let s = self.seq_counter;
        self.seq_counter = self.seq_counter.wrapping_add(1);
        s
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_cache_insert_and_get() {
        let mut cache = SchemaCache::new();
        let entry = SchemaEntry {
            name: "ping".to_string(),
            cmd_id: 0x0001,
            args_schema: vec![],
            ret_schema: vec![],
        };
        cache.insert(entry);
        assert!(cache.get(0x0001).is_some());
        assert!(cache.get(0x0002).is_none());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn schema_cache_invalidate() {
        let mut cache = SchemaCache::new();
        cache.insert(SchemaEntry {
            name: "ping".to_string(),
            cmd_id: 0x0001,
            args_schema: vec![],
            ret_schema: vec![],
        });
        cache.invalidate(0x0001);
        assert!(cache.is_empty());
    }

    #[test]
    fn call_raw_rejects_oversized_args() {
        struct NullIo;
        impl std::io::Read for NullIo {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }
        impl std::io::Write for NullIo {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut client = TelepathClient::new(NullIo);
        let oversized = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        assert!(matches!(
            client.call_raw(0x0001, &oversized),
            Err(HostError::PayloadTooLarge)
        ));
    }
}
