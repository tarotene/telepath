//! Integration test: full COBS+postcard round-trip over an in-memory loopback.
//!
//! Mirrors the `examples/loopback-demo` scenario but as a `cargo test`
//! so that regressions surface with clear failure output rather than
//! a timed-out process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

use telepath_client::TelepathClient;
use telepath_server::transport::Transport;
use telepath_server::{CommandMetadata, DispatchError, TelepathServer};
use telepath_wire::framing::MAX_FRAME_SIZE;

// ---------------------------------------------------------------------------
// Inline loopback transport (not shared with the binary crate)
// ---------------------------------------------------------------------------

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

impl Transport for FwSide {
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
        use std::sync::mpsc::TrySendError;
        let mut n = 0;
        for &b in buf {
            match self.tx.try_send(b) {
                Ok(()) => n += 1,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return n,
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

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

fn ping_shim(
    _input: &[u8],
    output: &mut [u8],
    _resources: &telepath_server::ResourceRegistry,
) -> Result<usize, DispatchError> {
    let val: u32 = 0xDEAD_BEEF;
    let written = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
    Ok(written.len())
}

fn noop_schema(_out: &mut [u8]) -> Result<usize, ()> {
    Ok(0)
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping",
    id: 0x0001,
    invoke: ping_shim,
    args_schema: noop_schema,
    ret_schema: noop_schema,
    arg_names: "",
}];

#[test]
fn ping_round_trip_over_loopback() {
    let (fw_t, host_t) = make_pair();
    let running = Arc::new(AtomicBool::new(true));

    let running_fw = Arc::clone(&running);
    let fw_handle = thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw_t, &COMMANDS);
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            thread::yield_now();
        }
    });

    let mut client = TelepathClient::new(host_t);
    let payload = client.call_raw(0x0001, &[]).expect("call_raw failed");
    let val: u32 = postcard::from_bytes(&payload).expect("deserialize failed");

    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");

    assert_eq!(val, 0xDEAD_BEEF);
}

fn fake_app_error_server(fw: FwSide) {
    use telepath_wire::framing::{cobs_decode, rzcobs_encode};
    use telepath_wire::{AppErrorPayload, PacketType, Request, Response, ResponseStatus};

    // Host→Target is COBS-framed; read until the 0x00 delimiter.
    let mut raw_frame: Vec<u8> = Vec::new();
    loop {
        let b = fw.rx.recv().expect("channel closed before delimiter");
        if b == 0x00 {
            break;
        }
        raw_frame.push(b);
    }

    // COBS decode (strip delimiter before passing in).
    let mut decoded_req = [0u8; 512];
    let req_n = cobs_decode(&raw_frame, &mut decoded_req).expect("cobs decode failed");
    let req: Request<'_> =
        postcard::from_bytes(&decoded_req[..req_n]).expect("request deserialize failed");

    // Build an AppError response that mirrors the accepted seq_no.
    let app_err = AppErrorPayload {
        code: 42,
        message: "test error",
    };
    let mut err_buf = [0u8; 64];
    let err_n =
        telepath_wire::encode_app_error(&app_err, &mut err_buf).expect("encode_app_error failed");

    let resp = Response {
        kind: PacketType::Response,
        seq_no: req.seq_no,
        status: ResponseStatus::AppError,
        payload: &err_buf[..err_n],
    };
    let mut resp_buf = [0u8; 512];
    let resp_ser = postcard::to_slice(&resp, &mut resp_buf).expect("response serialize failed");

    // Target→Host is rzCOBS-framed; rzcobs_encode includes the 0x00 delimiter.
    let mut frame_buf = [0u8; 1024];
    let frame_n = rzcobs_encode(resp_ser, &mut frame_buf).expect("rzcobs encode failed");
    for &b in &frame_buf[..frame_n] {
        fw.tx.send(b).expect("send failed");
    }
}

#[test]
fn app_error_round_trip_over_loopback() {
    let (fw_t, host_t) = make_pair();
    let fw_handle = thread::spawn(move || {
        fake_app_error_server(fw_t);
    });

    let mut client = TelepathClient::new(host_t);
    let result = client.call_raw(0x0001, &[]);

    fw_handle.join().expect("fw thread panicked");

    match result {
        Err(telepath_client::HostError::AppError { code, message }) => {
            assert_eq!(code, 42);
            assert_eq!(message, "test error");
        }
        other => {
            panic!("expected AppError {{ code: 42, message: \"test error\" }}, got: {other:?}")
        }
    }
}

#[test]
fn typed_call_ping_round_trip() {
    let (fw_t, host_t) = make_pair();
    let running = Arc::new(AtomicBool::new(true));

    let running_fw = Arc::clone(&running);
    let fw_handle = thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw_t, &COMMANDS);
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            thread::yield_now();
        }
    });

    let mut client = TelepathClient::new(host_t);
    client.discover().expect("discover failed");
    let ping_id = client
        .cmd_id_by_name("ping")
        .expect("ping not found in schema cache");
    let val: u32 = client
        .call::<(), u32>(ping_id, &())
        .expect("typed call failed");

    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");

    assert_eq!(val, 0xDEAD_BEEF);
}
