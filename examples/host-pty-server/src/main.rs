//! Host-side Telepath server over a PTY pair — no hardware required.
//!
//! Opens a PTY with `openpty(3)`, prints the slave device path to stdout so
//! that a test harness or human operator can connect a `telepath-shell
//! --features serial --port <path>` client, then runs `TelepathServer` on
//! the master end in a blocking loop.
//!
//! This is structurally identical to `examples/nrf52840-ping`: both are
//! server-side deployments that expose the same registered `#[command]` set;
//! they differ only in the physical transport layer.
//!
//! # Usage
//!
//! ```text
//! cargo run -p host-pty-server
//! # In another terminal, using the printed path:
//! cd tools/telepath-shell && cargo run --no-default-features --features serial -- --port /dev/pts/N --exec ping
//! ```

use heapless::Vec as HVec;
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    unistd::ttyname,
};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    thread,
    time::Duration,
};
use telepath_server::{command, TelepathServer};

// ---------------------------------------------------------------------------
// Commands served by this host-side deployment.
// These mirror the commands in examples/nrf52840-ping so the same test
// harness and MCP tooling can exercise both targets.
// ---------------------------------------------------------------------------

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

#[command]
fn crc32(payload: HVec<u8, 128>) -> u32 {
    crc32_iso_hdlc(&payload)
}

#[command]
fn echo(payload: HVec<u8, 128>) -> HVec<u8, 128> {
    payload
}

// ---------------------------------------------------------------------------
// PTY transport adapter
// ---------------------------------------------------------------------------

/// Wraps the PTY master `File` as a `telepath_server::transport::Transport`.
///
/// The master fd is set to O_NONBLOCK so `read` never stalls the server poll
/// loop. `write` remains blocking (PTY writes are fast in practice).
struct PtyTransport {
    master: File,
}

impl telepath_server::transport::Transport for PtyTransport {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        match self.master.read(buf) {
            Ok(n) => n,
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                0
            }
            Err(_) => 0,
        }
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        self.master.write(buf).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let pty = openpty(None, None)?;

    // Determine slave device path before consuming the slave fd.
    let slave_path = ttyname(&pty.slave)?;

    // Keep the slave fd open for the lifetime of the server process.
    // Without this, the master side receives EIO as soon as the last
    // slave reader closes (e.g. between test runs).
    let _slave_keeper = pty.slave;

    // Set master non-blocking so TelepathServer::poll() never blocks.
    fcntl(pty.master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;

    // Advertise the slave path to stdout for the test harness.
    println!("HOST_PTY_SERVER_PATH={}", slave_path.display());
    io::stdout().flush()?;

    let master_file: File = pty.master.into();
    let transport = PtyTransport {
        master: master_file,
    };
    let mut server = TelepathServer::<_, 512>::new(transport, telepath_server::commands());

    loop {
        server.poll();
        // Yield briefly so the OS schedules client I/O between polls.
        thread::sleep(Duration::from_millis(1));
    }
}
