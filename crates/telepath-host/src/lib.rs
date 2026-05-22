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

use telepath_wire::{
    framing::MAX_FRAME_SIZE, DiscoveryEntry, DiscoveryPage, DiscoveryRequest, CMD_ID_DISCOVERY,
    MAX_PAYLOAD_SIZE,
};

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
    /// A discovery page returned zero entries while `offset < total`, indicating
    /// a buggy or misbehaving firmware to prevent an infinite pagination loop.
    DiscoveryStalled,
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

    /// Run the Command Discovery Protocol, paginating until all commands are cached.
    ///
    /// Issues [`CMD_ID_DISCOVERY`] requests with successive `offset` values until
    /// `offset >= total`. Populates the schema cache from all pages. Returns the
    /// total number of commands discovered across all pages.
    ///
    /// The cache is fully reset at the start so repeated calls reflect the
    /// latest firmware state. `args_schema` and `ret_schema` are populated as
    /// opaque postcard-encoded `postcard_schema::schema::NamedType` bytes.
    ///
    /// Returns [`HostError::DiscoveryStalled`] if a page returns zero entries
    /// while `offset < total`, guarding against infinite loops on buggy firmware.
    pub fn discover(&mut self) -> Result<usize, HostError> {
        self.schema_cache = SchemaCache::new();
        let mut offset = 0u16;
        loop {
            let req_payload = postcard::to_allocvec(&DiscoveryRequest { offset })
                .map_err(|_| HostError::SerdeError)?;
            let raw = self.call_raw(CMD_ID_DISCOVERY, &req_payload)?;
            let page: DiscoveryPage<'_> =
                postcard::from_bytes(&raw).map_err(|_| HostError::SerdeError)?;
            let (count, mut rest): (u32, &[u8]) =
                postcard::take_from_bytes(page.entries).map_err(|_| HostError::SerdeError)?;
            for _ in 0..count {
                let (entry, next): (DiscoveryEntry<'_>, &[u8]) =
                    postcard::take_from_bytes(rest).map_err(|_| HostError::SerdeError)?;
                self.schema_cache.insert(SchemaEntry {
                    name: entry.name.to_owned(),
                    cmd_id: entry.id,
                    args_schema: entry.args_schema.to_vec(),
                    ret_schema: entry.ret_schema.to_vec(),
                });
                rest = next;
            }
            offset = offset.saturating_add(count as u16);
            if offset >= page.total {
                break;
            }
            if count == 0 {
                return Err(HostError::DiscoveryStalled);
            }
        }
        Ok(self.schema_cache.len())
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

    /// Mutable access to the underlying transport (e.g. for side-channel operations
    /// like draining debug logs on an RTT adapter).
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
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

    /// End-to-end discover() round-trip using an in-process blocking pipe.
    ///
    /// Spawns a `TelepathServer` on a background thread and calls
    /// `client.discover()` from the test thread to verify that `SchemaCache`
    /// is populated with the server's registered commands.
    #[test]
    fn discover_roundtrip() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
        use std::sync::Arc;
        use std::thread;
        use telepath_firmware::transport::Transport as FwTransport;
        use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};
        use telepath_wire::framing::MAX_FRAME_SIZE;

        fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
            let s = postcard::to_slice(&0xDEAD_BEEFu32, output)
                .map_err(|_| DispatchError::SerializeError)?;
            Ok(s.len())
        }
        fn noop_schema(_out: &mut [u8]) -> Result<usize, ()> {
            Ok(0)
        }
        static CMDS: [CommandMetadata; 1] = [CommandMetadata {
            name: "ping",
            id: 0x0001,
            invoke: ping_shim,
            args_schema: noop_schema,
            ret_schema: noop_schema,
        }];

        // --- Inline blocking pipe (mirrors host-emulator/src/loopback.rs) ---
        struct FwSide {
            rx: Receiver<u8>,
            tx: SyncSender<u8>,
        }
        struct HostSide {
            rx: Receiver<u8>,
            tx: SyncSender<u8>,
        }

        fn make_pair() -> (FwSide, HostSide) {
            let cap = MAX_FRAME_SIZE * 2;
            let (h2f_tx, h2f_rx) = sync_channel::<u8>(cap);
            let (f2h_tx, f2h_rx) = sync_channel::<u8>(cap);
            (
                FwSide {
                    rx: h2f_rx,
                    tx: f2h_tx,
                },
                HostSide {
                    rx: f2h_rx,
                    tx: h2f_tx,
                },
            )
        }

        impl FwTransport for FwSide {
            fn read(&mut self, buf: &mut [u8]) -> usize {
                let mut n = 0;
                while n < buf.len() {
                    match self.rx.try_recv() {
                        Ok(b) => {
                            buf[n] = b;
                            n += 1;
                        }
                        Err(_) => break,
                    }
                }
                n
            }
            fn write(&mut self, buf: &[u8]) -> usize {
                let mut n = 0;
                for &b in buf {
                    match self.tx.try_send(b) {
                        Ok(()) => n += 1,
                        Err(_) => return n,
                    }
                }
                n
            }
        }

        impl std::io::Read for HostSide {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if buf.is_empty() {
                    return Ok(0);
                }
                let first = self.rx.recv().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fw disconnected")
                })?;
                buf[0] = first;
                let mut n = 1;
                while n < buf.len() {
                    match self.rx.try_recv() {
                        Ok(b) => {
                            buf[n] = b;
                            n += 1;
                        }
                        Err(_) => break,
                    }
                }
                Ok(n)
            }
        }
        impl std::io::Write for HostSide {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                for &b in buf {
                    self.tx.send(b).map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "fw disconnected")
                    })?;
                }
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // --- End of inline pipe ---

        // RAII guard: stops and joins the fw thread even if the test panics.
        struct FwGuard {
            running: Arc<AtomicBool>,
            handle: Option<thread::JoinHandle<()>>,
        }
        impl Drop for FwGuard {
            fn drop(&mut self) {
                self.running.store(false, Ordering::Release);
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
            }
        }

        let (fw_t, host_t) = make_pair();
        let running = Arc::new(AtomicBool::new(true));
        let running_fw = Arc::clone(&running);
        let fw_handle = thread::spawn(move || {
            let mut server = TelepathServer::<_, 512>::new(fw_t, &CMDS);
            while running_fw.load(Ordering::Acquire) {
                server.poll();
                thread::yield_now();
            }
        });
        let _guard = FwGuard {
            running: Arc::clone(&running),
            handle: Some(fw_handle),
        };

        let mut client = TelepathClient::new(host_t);
        let n = client.discover().expect("discover failed");
        assert_eq!(n, 1, "expected exactly 1 registered command (ping)");
        let entry = client
            .schema_cache()
            .get(0x0001)
            .expect("ping not in SchemaCache");
        assert_eq!(entry.name, "ping");
        assert_eq!(entry.cmd_id, 0x0001);
        // CMDS uses noop_schema so schema bytes are empty — that's expected for
        // hand-written fixtures; real #[command] functions produce non-empty bytes.
        // Verify the host faithfully stored whatever the server sent.
        assert_eq!(entry.args_schema, Vec::<u8>::new());
        assert_eq!(entry.ret_schema, Vec::<u8>::new());
    }

    /// Server-side round-trip: manually encode a request, feed to server, decode the response.
    /// Does not exercise call_raw.
    #[test]
    fn server_ping_roundtrip() {
        use telepath_firmware::transport::Transport as FwTransport;
        use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};

        fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
            let val: u32 = 0xDEAD_BEEF;
            let s = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
            Ok(s.len())
        }

        fn noop_schema_sp(_out: &mut [u8]) -> Result<usize, ()> {
            Ok(0)
        }
        static CMDS: [CommandMetadata; 1] = [CommandMetadata {
            name: "ping",
            id: 0x0001,
            invoke: ping_shim,
            args_schema: noop_schema_sp,
            ret_schema: noop_schema_sp,
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
        let mut server = TelepathServer::<PipeTransport, 512>::new(server_transport, &CMDS);

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
