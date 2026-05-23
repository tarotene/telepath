#![cfg(unix)]

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::openpty;
use nix::sys::termios;
use serde_json::json;
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use telepath_client::TelepathClient;
use telepath_mcp_server::bridge;
use telepath_server::{command, transport::Transport as FwTransport, TelepathServer};

// ── demo command registered for this test ────────────────────────────────────

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

// ── fw-side file transport ────────────────────────────────────────────────────

struct FwFileTransport(File);

impl FwTransport for FwFileTransport {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        use std::io::Read;
        match self.0.read(buf) {
            Ok(n) => n,
            // WouldBlock: O_NONBLOCK + no data — signal "nothing available"
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => 0,
            Err(_) => 0,
        }
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        use std::io::Write;
        match self.0.write(buf) {
            Ok(n) => n,
            Err(_) => 0,
        }
    }
}

fn spawn_fw_file(fw: FwFileTransport) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let running = Arc::new(AtomicBool::new(true));
    let running_fw = Arc::clone(&running);
    let handle = std::thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw, telepath_server::commands());
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            std::thread::yield_now();
        }
    });
    (running, handle)
}

// ── test ─────────────────────────────────────────────────────────────────────

/// Exercises the full Telepath wire path (COBS framing + postcard serialization)
/// over a Unix pty pair, without any physical hardware. The slave fd is the
/// firmware side; the master fd is the host (TelepathClient) side.
#[tokio::test(flavor = "multi_thread")]
async fn serial_pty_round_trip() {
    // 1. Open a pty pair (nix 0.26: master/slave are RawFd).
    let pty = openpty(None, None).expect("openpty failed");
    let slave_raw_fd = pty.slave;
    let master_raw_fd = pty.master;

    // 2. Set slave to raw mode so the terminal line discipline does not mangle
    //    COBS frames (e.g. NUL stripping, NL→CRNL translation, ^C handling).
    let mut tios = termios::tcgetattr(slave_raw_fd).expect("tcgetattr failed");
    termios::cfmakeraw(&mut tios);
    termios::tcsetattr(slave_raw_fd, termios::SetArg::TCSANOW, &tios)
        .expect("tcsetattr failed");

    // 3. Set slave to O_NONBLOCK so FwTransport::read returns 0 instead of
    //    blocking, enabling clean AtomicBool-driven shutdown.
    fcntl(slave_raw_fd, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
        .expect("fcntl O_NONBLOCK failed");

    // 4. Wrap raw fds as Files.
    let master = unsafe { File::from_raw_fd(master_raw_fd) };
    let slave = unsafe { File::from_raw_fd(slave_raw_fd) };

    // 5. Spawn the firmware thread.
    let (running, fw_handle) = spawn_fw_file(FwFileTransport(slave));

    // 6. Build a TelepathClient on the master fd and run the full bridge stack.
    let mut client = TelepathClient::new(master);
    let n = client.discover().expect("discover failed");
    assert!(n >= 1, "expected at least 1 command, got {n}");

    let entry = client
        .schema_cache()
        .iter()
        .find(|e| e.name == "ping")
        .expect("ping not in schema cache")
        .clone();

    let args_schema = entry.decoded_args_schema().expect("decode args schema");
    let ret_schema = entry.decoded_ret_schema().expect("decode ret schema");

    let result =
        bridge::invoke(&mut client, entry.cmd_id, &args_schema, &ret_schema, &json!({}))
            .await
            .expect("bridge::invoke failed");

    // 0xDEADBEEF = 3735928559
    assert_eq!(result, json!(0xDEAD_BEEFu32));

    // 7. Shut down the firmware thread cleanly.
    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");
}
