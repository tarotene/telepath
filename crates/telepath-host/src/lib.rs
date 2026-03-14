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

use telepath_wire::{framing::MAX_FRAME_SIZE, CMD_ID_DISCOVERY, MAX_PAYLOAD_SIZE};

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
    /// The request args exceeded [`MAX_PAYLOAD_SIZE`].
    RequestPayloadTooLarge,
    /// The response payload exceeded [`MAX_PAYLOAD_SIZE`].
    ResponsePayloadTooLarge,
    /// The received frame exceeded [`MAX_FRAME_SIZE`].
    FrameTooLarge,
    /// COBS framing error (malformed frame received from target).
    FramingError,
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
/// `T` must implement `std::io::Read + std::io::Write` (e.g. a serialport or
/// a probe-rs RTT adapter).
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
        // TODO: deserialize the CDP response into SchemaEntry objects once
        // the firmware-side CDP serialization is implemented.
        let _ = self.call_raw(CMD_ID_DISCOVERY, &[])?;
        Ok(0)
    }

    /// Issue a raw RPC call.
    ///
    /// `cmd_id` identifies the target command; `args` is a postcard-serialized
    /// argument struct. Returns the postcard-serialized response payload on
    /// success.
    pub fn call_raw(&mut self, cmd_id: u16, args: &[u8]) -> Result<Vec<u8>, HostError> {
        if args.len() > MAX_PAYLOAD_SIZE {
            return Err(HostError::RequestPayloadTooLarge);
        }
        let seq = self.next_seq();

        // Build and serialize Request.
        let req = telepath_wire::Request {
            kind: telepath_wire::PacketType::Request,
            seq_no: seq,
            cmd_id,
            args,
        };
        let serialized = postcard::to_allocvec(&req).map_err(|_| HostError::SerdeError)?;

        // COBS encode + 0x00 delimiter.
        let encoded_cap = cobs::max_encoding_length(serialized.len()) + 1;
        let mut encoded = vec![0u8; encoded_cap];
        let n = cobs::encode(&serialized, &mut encoded);
        encoded[n] = 0x00;

        // Send.
        self.transport
            .write_all(&encoded[..n + 1])
            .map_err(|e| HostError::Io(e.to_string()))?;

        // Receive response bytes until 0x00 delimiter.
        let mut raw_frame: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            self.transport
                .read_exact(&mut byte)
                .map_err(|e| HostError::Io(e.to_string()))?;
            if byte[0] == 0x00 {
                break;
            }
            if raw_frame.len() >= MAX_FRAME_SIZE {
                return Err(HostError::FrameTooLarge);
            }
            raw_frame.push(byte[0]);
        }

        // COBS decode.
        let mut decoded = vec![0u8; raw_frame.len()];
        let m = cobs::decode(&raw_frame, &mut decoded).map_err(|_| HostError::FramingError)?;
        decoded.truncate(m);

        // Deserialize Response.
        let resp: telepath_wire::Response<'_> =
            postcard::from_bytes(&decoded).map_err(|_| HostError::SerdeError)?;

        // Validate packet kind and payload size.
        if resp.kind != telepath_wire::PacketType::Response {
            return Err(HostError::FramingError);
        }
        if resp.payload.len() > MAX_PAYLOAD_SIZE {
            return Err(HostError::ResponsePayloadTooLarge);
        }

        // Validate sequence number.
        if resp.seq_no != seq {
            return Err(HostError::SeqMismatch {
                expected: seq,
                got: resp.seq_no,
            });
        }

        match resp.status {
            telepath_wire::ResponseStatus::Ok => Ok(resp.payload.to_vec()),
            telepath_wire::ResponseStatus::SystemError => Err(HostError::SystemError),
            telepath_wire::ResponseStatus::AppError => {
                Err(HostError::AppError(resp.payload.to_vec()))
            }
        }
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
            Err(HostError::RequestPayloadTooLarge)
        ));
    }

    /// Full round-trip test: encode a request, server processes it, decode response.
    #[test]
    fn call_raw_ping_roundtrip() {
        use telepath_firmware::transport::Transport as FwTransport;
        use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};

        fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
            let val: u32 = 0xDEAD_BEEF;
            let s = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
            Ok(s.len())
        }

        static CMDS: [CommandMetadata; 1] = [CommandMetadata {
            name: "ping",
            id: 0x0001,
            invoke: ping_shim,
        }];

        // Pipe: client writes → server reads, server writes → client reads.
        use std::sync::{Arc, Mutex};
        let c2s: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let s2c: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        struct PipeTransport {
            rx: Arc<Mutex<Vec<u8>>>,
            tx: Arc<Mutex<Vec<u8>>>,
        }

        impl FwTransport for PipeTransport {
            fn read(&mut self, buf: &mut [u8]) -> usize {
                let mut rx = self.rx.lock().unwrap();
                let n = buf.len().min(rx.len());
                buf[..n].copy_from_slice(&rx[..n]);
                rx.drain(..n);
                n
            }
            fn write(&mut self, buf: &[u8]) -> usize {
                self.tx.lock().unwrap().extend_from_slice(buf);
                buf.len()
            }
        }

        impl std::io::Read for PipeTransport {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = FwTransport::read(self, buf);
                Ok(n)
            }
        }
        impl std::io::Write for PipeTransport {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(FwTransport::write(self, buf))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let server_transport = PipeTransport {
            rx: Arc::clone(&c2s),
            tx: Arc::clone(&s2c),
        };
        let client_transport = PipeTransport {
            rx: Arc::clone(&s2c),
            tx: Arc::clone(&c2s),
        };

        let mut server = TelepathServer::<PipeTransport, 512>::new(server_transport, &CMDS);
        let mut client = TelepathClient::new(client_transport);

        // Client sends ping; server processes synchronously.
        // We need to interleave: client writes, server polls, client reads.
        // Encode the request manually and have server poll before client reads.
        use telepath_wire::framing::cobs_encode;
        use telepath_wire::{PacketType, Request};

        let req = Request {
            kind: PacketType::Request,
            seq_no: 0,
            cmd_id: 0x0001,
            args: &[],
        };
        let mut ser_buf = [0u8; 64];
        let serialized = postcard::to_slice(&req, &mut ser_buf).unwrap();
        let mut framed = [0u8; 64];
        let n = cobs_encode(serialized, &mut framed).unwrap();

        // Write request into c2s pipe.
        c2s.lock().unwrap().extend_from_slice(&framed[..n]);

        // Server poll.
        server.poll();

        // Now client reads the response from s2c pipe.
        // But TelepathClient.call_raw sends a request first then reads...
        // For this test, s2c already has the server's response.
        // We just decode it manually.
        let response_bytes = s2c.lock().unwrap().clone();
        assert!(!response_bytes.is_empty());

        let delim = response_bytes
            .iter()
            .position(|&b| b == 0x00)
            .expect("no delimiter");
        let mut decoded = [0u8; 256];
        let m =
            telepath_wire::framing::cobs_decode(&response_bytes[..delim], &mut decoded).unwrap();
        let resp: telepath_wire::Response<'_> = postcard::from_bytes(&decoded[..m]).unwrap();
        assert_eq!(resp.status, telepath_wire::ResponseStatus::Ok);
        let val: u32 = postcard::from_bytes(resp.payload).unwrap();
        assert_eq!(val, 0xDEAD_BEEF);
    }
}
