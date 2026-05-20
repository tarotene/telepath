//! In-process Telepath emulator — no hardware required.
//!
//! Spawns a `TelepathServer` on one OS thread and drives it from a
//! `TelepathClient` on the main thread, using a pair of `std::sync::mpsc`
//! byte channels as the "wire". The full COBS framing + postcard path runs
//! exactly as it does on real hardware; only the bottom-most byte stream
//! is swapped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};
use telepath_host::{HostError, TelepathClient};

mod loopback;

const CMD_PING: u16 = 0x0001;

/// Ping handler: returns the fixed sentinel value `0xDEAD_BEEF` as a `u32`.
///
/// `ShimFn` signature: `fn(input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError>`
/// Write serialized bytes into `output` and return the number of bytes written.
fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
    let val: u32 = 0xDEAD_BEEF;
    let written = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
    Ok(written.len())
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping",
    id: CMD_PING,
    invoke: ping_shim,
}];

fn main() -> Result<(), HostError> {
    let (fw_transport, host_transport) = loopback::make_pair();
    let running = Arc::new(AtomicBool::new(true));

    // Firmware thread: poll the server in a tight loop.
    let running_fw = Arc::clone(&running);
    let fw_handle = thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw_transport, &COMMANDS);
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            // yield_now lets the OS schedule the host thread while the
            // firmware channel is empty, avoiding a 100% CPU spin.
            thread::yield_now();
        }
    });

    // Host thread (main): send one request, decode the response, print it.
    let mut client = TelepathClient::new(host_transport);
    let payload = client.call_raw(CMD_PING, &[])?;
    let val: u32 = postcard::from_bytes(&payload).expect("ping returned invalid u32");
    println!("ping -> 0x{:08X}", val);

    // Signal the firmware thread to exit and wait for it.
    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");

    Ok(())
}
