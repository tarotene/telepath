//! Host-side Telepath library.
//!
//! Provides [`TelepathClient`] for issuing RPC calls to a target device, and
//! [`SchemaCache`] for caching discovered command schemas keyed by `cmd_id`.
//!
//! # Usage
//!
//! ```rust,ignore
//! use telepath_client::TelepathClient;
//!
//! let mut client = TelepathClient::new(serial_port);
//! client.discover().unwrap();
//! let ping_id = client.cmd_id_by_name("ping").unwrap();
//! let result: u32 = client.call::<(), u32>(ping_id, &()).unwrap();
//! ```

#[cfg(feature = "rtt")]
pub mod rtt_transport;
#[cfg(feature = "serial")]
pub mod serial_transport;

use telepath_wire::{
    framing::{rzcobs_decode, MAX_FRAME_SIZE},
    DiscoveryEntry, DiscoveryPage, DiscoveryRequest, CMD_ID_DISCOVERY, MAX_PAYLOAD_SIZE,
};

// ---------------------------------------------------------------------------
// HostTransportExt
// ---------------------------------------------------------------------------

/// Extension trait for host-side transports that support read deadlines and
/// optional debug-log draining (RTT channel 0).
///
/// All methods have no-op defaults so that simpler transports (e.g. serial
/// ports, in-memory pipes) do not need to implement them.
pub trait HostTransportExt: std::io::Read + std::io::Write {
    /// Set a per-call read timeout: each `Read::read()` invocation waits at most
    /// `timeout` for data before returning `TimedOut`. Calling again updates the
    /// timeout for future reads.
    fn set_read_deadline(&mut self, _timeout: std::time::Duration) {}

    /// Disarm the read deadline; reads block indefinitely again.
    fn clear_read_deadline(&mut self) {}

    /// Drain an out-of-band debug log channel to `sink`. Non-blocking no-op
    /// by default; RTT transports override this for channel 0 drain.
    fn drain_debug_logs(&mut self, _sink: &mut dyn std::io::Write) {}

    /// Discard stale frames in the RPC receive buffer left over from a
    /// previous session. No-op by default; RTT transports override this.
    fn drain_rpc_rx(&mut self) {}
}

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
    /// The target reported an application-level error with a decoded payload.
    ///
    /// Inspect `code` and `message` to understand the failure. `code` is
    /// application-defined; `0` means "unspecified application error".
    AppError { code: u16, message: String },
    /// postcard serialization or deserialization failed; carries the underlying cause.
    SerdeError(postcard::Error),
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
    /// A discovery page returned unexpected metadata (wrong echoed offset or
    /// inconsistent total), indicating a misbehaving firmware.
    DiscoveryProtocolError,
}

impl From<postcard::Error> for HostError {
    fn from(e: postcard::Error) -> Self {
        HostError::SerdeError(e)
    }
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
    /// Argument names in declaration order, e.g. `["a", "b"]` for `fn foo(a: i32, b: i32)`.
    /// Empty for zero-argument commands.
    pub arg_names: Vec<String>,
}

impl SchemaEntry {
    /// Decode the raw argument-schema bytes into an `OwnedNamedType`.
    pub fn decoded_args_schema(
        &self,
    ) -> Result<postcard_schema::schema::owned::OwnedNamedType, HostError> {
        Ok(postcard::from_bytes(&self.args_schema)?)
    }

    /// Decode the raw return-schema bytes into an `OwnedNamedType`.
    pub fn decoded_ret_schema(
        &self,
    ) -> Result<postcard_schema::schema::owned::OwnedNamedType, HostError> {
        Ok(postcard::from_bytes(&self.ret_schema)?)
    }
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

    /// Remove all cached entries.
    ///
    /// [`TelepathClient::discover`] and [`TelepathClient::rediscover`] reset the
    /// cache by assigning a fresh [`SchemaCache::new`] rather than calling this
    /// method, so manual `clear()` is unnecessary for routine rediscovery.
    /// Exposed for callers that manage the cache directly and need to reset it
    /// without running CDP.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SchemaEntry> {
        self.entries.values()
    }
}

// ---------------------------------------------------------------------------
// HostMetricsSnapshot (profile feature)
// ---------------------------------------------------------------------------

/// Accumulated host-side framing timing counters.
///
/// Returned by [`TelepathClient::take_host_metrics`] and atomically reset.
/// All timing values are in nanoseconds; byte counts are raw bytes processed.
#[cfg(feature = "profile")]
#[derive(Debug, Clone, Copy, Default)]
pub struct HostMetricsSnapshot {
    /// Total ns spent in `cobs::encode` across all calls since last reset.
    pub encode_ns: u64,
    /// Total ns spent in `cobs::decode` across all calls since last reset.
    pub decode_ns: u64,
    /// Total bytes COBS-encoded (after postcard serialization).
    pub encoded_bytes: u64,
    /// Total bytes COBS-decoded (raw frame bytes fed to decoder).
    pub decoded_bytes: u64,
    /// Number of complete round-trips measured.
    pub sample_count: u64,
}

// ---------------------------------------------------------------------------
// TelepathClient
// ---------------------------------------------------------------------------

/// RPC client that communicates with a [`telepath_server::TelepathServer`].
///
/// `T` must implement `std::io::Read + std::io::Write` (e.g. a serialport or
/// a probe-rs RTT adapter).
pub struct TelepathClient<T> {
    transport: T,
    schema_cache: SchemaCache,
    seq_counter: u16,
    /// Bytes read from the transport but not yet consumed by a `call_raw`.
    /// Allows `call_raw` to read in chunks rather than one byte at a time,
    /// saving a USB roundtrip per byte on slow transports like RTT over J-Link.
    read_buf: Vec<u8>,
    #[cfg(feature = "profile")]
    host_metrics: HostMetricsSnapshot,
}

impl<T: std::io::Read + std::io::Write> TelepathClient<T> {
    /// Create a new client wrapping the given transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            schema_cache: SchemaCache::new(),
            seq_counter: 0,
            read_buf: Vec::new(),
            #[cfg(feature = "profile")]
            host_metrics: HostMetricsSnapshot::default(),
        }
    }

    /// Return the accumulated host-side framing metrics and reset all counters to zero.
    #[cfg(feature = "profile")]
    pub fn take_host_metrics(&mut self) -> HostMetricsSnapshot {
        let snap = self.host_metrics;
        self.host_metrics = HostMetricsSnapshot::default();
        snap
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
        let mut expected_total: Option<u16> = None;
        loop {
            let req_payload = postcard::to_allocvec(&DiscoveryRequest { offset })?;
            let raw = self.call_raw(CMD_ID_DISCOVERY, &req_payload)?;
            let page: DiscoveryPage<'_> = postcard::from_bytes(&raw)?;

            // Validate that the firmware echoed the offset we requested.
            if page.offset != offset {
                return Err(HostError::DiscoveryProtocolError);
            }
            // Validate that page.total is consistent across all pages.
            match expected_total {
                None => expected_total = Some(page.total),
                Some(t) if t != page.total => return Err(HostError::DiscoveryProtocolError),
                _ => {}
            }

            let (count, mut rest): (u32, &[u8]) = postcard::take_from_bytes(page.entries)?;
            for _ in 0..count {
                let (entry, next): (DiscoveryEntry<'_>, &[u8]) = postcard::take_from_bytes(rest)?;
                self.schema_cache.insert(SchemaEntry {
                    name: entry.name.to_owned(),
                    cmd_id: entry.id,
                    args_schema: entry.args_schema.to_vec(),
                    ret_schema: entry.ret_schema.to_vec(),
                    arg_names: if entry.arg_names.is_empty() {
                        vec![]
                    } else {
                        entry.arg_names.split(',').map(str::to_owned).collect()
                    },
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

    /// Re-run the Command Discovery Protocol from scratch.
    ///
    /// Equivalent to [`discover`](Self::discover) but makes the intent explicit:
    /// the caller is reconnecting to a (potentially updated) firmware and needs a
    /// fresh schema cache.
    ///
    /// Use this after a transport reconnect or firmware reflash to avoid stale
    /// `cmd_id` lookups. Automatic triggering on transport events is left to
    /// higher-level wrappers (e.g. `TelepathMcpServer::rediscover`).
    pub fn rediscover(&mut self) -> Result<usize, HostError> {
        self.discover()
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
        let serialized = postcard::to_allocvec(&req)?;

        // COBS encode + 0x00 delimiter.
        let encoded_cap = cobs::max_encoding_length(serialized.len()) + 1;
        let mut encoded = vec![0u8; encoded_cap];
        #[cfg(feature = "profile")]
        let t_enc = std::time::Instant::now();
        let n = cobs::encode(&serialized, &mut encoded);
        #[cfg(feature = "profile")]
        {
            let dt = t_enc.elapsed().as_nanos() as u64;
            self.host_metrics.encode_ns = self.host_metrics.encode_ns.saturating_add(dt);
            self.host_metrics.encoded_bytes = self
                .host_metrics
                .encoded_bytes
                .saturating_add(serialized.len() as u64);
        }
        encoded[n] = 0x00;

        // Send.
        self.transport
            .write_all(&encoded[..n + 1])
            .map_err(|e| HostError::Io(e.to_string()))?;

        // Receive response bytes until 0x00 delimiter (rzCOBS frame boundary).
        // Read in chunks to avoid a USB roundtrip per byte on slow transports.
        let mut raw_frame: Vec<u8> = Vec::new();
        'recv: loop {
            if let Some(pos) = self.read_buf.iter().position(|&b| b == 0x00) {
                if raw_frame.len() + pos >= MAX_FRAME_SIZE {
                    self.read_buf.drain(..=pos);
                    return Err(HostError::FrameTooLarge);
                }
                raw_frame.extend_from_slice(&self.read_buf[..pos]);
                self.read_buf.drain(..=pos);
                break 'recv;
            }
            if raw_frame.len() + self.read_buf.len() >= MAX_FRAME_SIZE {
                self.read_buf.clear();
                return Err(HostError::FrameTooLarge);
            }
            raw_frame.extend_from_slice(&self.read_buf);
            self.read_buf.clear();
            let mut chunk = [0u8; 256];
            let n = self
                .transport
                .read(&mut chunk)
                .map_err(|e| HostError::Io(e.to_string()))?;
            if n == 0 {
                return Err(HostError::Io("transport closed (EOF)".to_string()));
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
        }

        // rzCOBS decode (upstream framing; decoded length may exceed encoded length).
        let mut decoded = vec![0u8; MAX_FRAME_SIZE];
        #[cfg(feature = "profile")]
        let t_dec = std::time::Instant::now();
        let m = rzcobs_decode(&raw_frame, &mut decoded).map_err(|_| HostError::FramingError)?;
        #[cfg(feature = "profile")]
        {
            let dt = t_dec.elapsed().as_nanos() as u64;
            self.host_metrics.decode_ns = self.host_metrics.decode_ns.saturating_add(dt);
            self.host_metrics.decoded_bytes =
                self.host_metrics.decoded_bytes.saturating_add(m as u64);
            self.host_metrics.sample_count = self.host_metrics.sample_count.saturating_add(1);
        }
        decoded.truncate(m);

        // Deserialize Response.
        let resp: telepath_wire::Response<'_> = postcard::from_bytes(&decoded)?;

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
                let payload: telepath_wire::AppErrorPayload<'_> =
                    postcard::from_bytes(resp.payload)?;
                Err(HostError::AppError {
                    code: payload.code,
                    message: payload.message.to_owned(),
                })
            }
        }
    }

    /// Typed RPC call.
    ///
    /// Postcard-serializes `args`, delegates to [`Self::call_raw`], and
    /// postcard-deserializes the response payload into `Ret`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// client.discover()?;
    /// let ping_id = client.cmd_id_by_name("ping").unwrap();
    /// let result: u32 = client.call::<(), u32>(ping_id, &())?;
    /// ```
    pub fn call<Args, Ret>(&mut self, cmd_id: u16, args: &Args) -> Result<Ret, HostError>
    where
        Args: serde::Serialize,
        Ret: serde::de::DeserializeOwned,
    {
        let args_bytes = postcard::to_allocvec(args)?;
        let payload = self.call_raw(cmd_id, &args_bytes)?;
        let ret = postcard::from_bytes(&payload)?;
        Ok(ret)
    }

    /// Resolve a command name to its `cmd_id` using the populated schema cache.
    ///
    /// Returns `None` if [`Self::discover`] has not been called, or the name
    /// is not registered on the target.
    ///
    /// # Note
    ///
    /// If multiple commands share `name` (possible when signatures differ),
    /// the returned id is implementation-defined. Disambiguation is tracked in
    /// [#175](https://github.com/tarotene/telepath/issues/175).
    pub fn cmd_id_by_name(&self, name: &str) -> Option<u16> {
        self.schema_cache
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.cmd_id)
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
            arg_names: vec![],
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
            arg_names: vec![],
        });
        cache.invalidate(0x0001);
        assert!(cache.is_empty());
    }

    #[test]
    fn schema_cache_clear_removes_all_entries() {
        let mut cache = SchemaCache::new();
        cache.insert(SchemaEntry {
            name: "a".to_string(),
            cmd_id: 0x0001,
            args_schema: vec![],
            ret_schema: vec![],
            arg_names: vec![],
        });
        cache.insert(SchemaEntry {
            name: "b".to_string(),
            cmd_id: 0x0002,
            args_schema: vec![],
            ret_schema: vec![],
            arg_names: vec![],
        });
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty(), "clear() must remove all entries");
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
        use telepath_server::transport::Transport as FwTransport;
        use telepath_server::{CommandMetadata, DispatchError, TelepathServer};
        use telepath_wire::framing::MAX_FRAME_SIZE;

        fn ping_shim(
            _input: &[u8],
            output: &mut [u8],
            _resources: &telepath_server::ResourceRegistry,
        ) -> Result<usize, DispatchError> {
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
            arg_names: "",
        }];

        // --- Inline blocking pipe (mirrors loopback-demo/src/loopback.rs) ---
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

        // rediscover() must reset the cache and re-populate it.
        let n2 = client.rediscover().expect("rediscover failed");
        assert_eq!(n2, 1, "rediscover must still find 1 command");
        assert!(
            client.schema_cache().get(0x0001).is_some(),
            "rediscover must re-populate ping in the cache"
        );
    }

    // Golden fixtures: postcard::to_allocvec(<T as Schema>::SCHEMA)
    // Captured 2026-05-23 from postcard-schema 0.2.5
    const FIXTURE_UNIT: &[u8] = &[0x02, 0x28, 0x29, 0x13];
    const FIXTURE_U32: &[u8] = &[0x03, 0x75, 0x33, 0x32, 0x08];

    #[test]
    fn fixture_unit_matches_current_postcard_schema() {
        use postcard_schema::Schema;
        let expected: Vec<u8> = postcard::to_allocvec(&<() as Schema>::SCHEMA).expect("serialize");
        assert_eq!(
            FIXTURE_UNIT,
            expected.as_slice(),
            "FIXTURE_UNIT is out of sync — update it to: {:?}",
            expected
        );
    }

    #[test]
    fn fixture_u32_matches_current_postcard_schema() {
        use postcard_schema::Schema;
        let expected: Vec<u8> = postcard::to_allocvec(&<u32 as Schema>::SCHEMA).expect("serialize");
        assert_eq!(
            FIXTURE_U32,
            expected.as_slice(),
            "FIXTURE_U32 is out of sync — update it to: {:?}",
            expected
        );
    }

    #[test]
    fn schema_entry_decoded_args_schema_roundtrip() {
        use postcard_schema::schema::owned::OwnedNamedType;
        let entry = SchemaEntry {
            name: "ping".into(),
            cmd_id: 1,
            args_schema: FIXTURE_UNIT.to_vec(),
            ret_schema: FIXTURE_U32.to_vec(),
            arg_names: vec![],
        };
        let args: OwnedNamedType = entry.decoded_args_schema().expect("args decode");
        assert_eq!(args.name.as_ref() as &str, "()");
        let ret: OwnedNamedType = entry.decoded_ret_schema().expect("ret decode");
        assert_eq!(ret.name.as_ref() as &str, "u32");
    }

    #[test]
    fn schema_entry_decode_returns_err_on_garbage() {
        let entry = SchemaEntry {
            name: "x".into(),
            cmd_id: 1,
            args_schema: vec![0xFF, 0xFF, 0xFF],
            ret_schema: vec![],
            arg_names: vec![],
        };
        assert!(entry.decoded_args_schema().is_err());
    }

    #[test]
    fn schema_cache_iter_yields_inserted_entries() {
        let mut cache = SchemaCache::new();
        cache.insert(SchemaEntry {
            name: "a".into(),
            cmd_id: 0x0001,
            args_schema: vec![1],
            ret_schema: vec![],
            arg_names: vec![],
        });
        cache.insert(SchemaEntry {
            name: "b".into(),
            cmd_id: 0x0002,
            args_schema: vec![],
            ret_schema: vec![2],
            arg_names: vec![],
        });
        let mut names: Vec<&str> = cache.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn schema_cache_iter_is_empty_on_empty_cache() {
        assert_eq!(SchemaCache::new().iter().count(), 0);
    }

    /// Server-side round-trip: manually encode a request, feed to server, decode the response.
    /// Does not exercise call_raw.
    #[test]
    fn server_ping_roundtrip() {
        use telepath_server::transport::Transport as FwTransport;
        use telepath_server::{CommandMetadata, DispatchError, TelepathServer};

        fn ping_shim(
            _input: &[u8],
            output: &mut [u8],
            _resources: &telepath_server::ResourceRegistry,
        ) -> Result<usize, DispatchError> {
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
            arg_names: "",
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
        let mut decoded = [0u8; 512];
        let m =
            telepath_wire::framing::rzcobs_decode(&response_bytes[..delim], &mut decoded).unwrap();
        let resp: telepath_wire::Response<'_> = postcard::from_bytes(&decoded[..m]).unwrap();
        assert_eq!(resp.status, telepath_wire::ResponseStatus::Ok);
        let val: u32 = postcard::from_bytes(resp.payload).unwrap();
        assert_eq!(val, 0xDEAD_BEEF);
    }
}
