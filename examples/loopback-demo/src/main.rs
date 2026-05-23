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

use telepath_client::{HostError, TelepathClient};
use telepath_server::{command, TelepathServer};

mod loopback;

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

fn main() -> Result<(), HostError> {
    let (fw_transport, host_transport) = loopback::make_pair();
    let running = Arc::new(AtomicBool::new(true));

    // Firmware thread: poll the server in a tight loop.
    let running_fw = Arc::clone(&running);
    let fw_handle = thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw_transport, telepath_server::commands());
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            // yield_now lets the OS schedule the host thread while the
            // firmware channel is empty, avoiding a 100% CPU spin.
            thread::yield_now();
        }
    });

    // Host thread (main): send one request, decode the response, print it.
    let mut client = TelepathClient::new(host_transport);
    let payload = client.call_raw(__TELEPATH_CMD_PING.id, &[])?;
    let val: u32 = postcard::from_bytes(&payload).expect("ping returned invalid u32");
    println!("ping -> 0x{:08X}", val);

    let n = client.discover()?;
    println!("discover -> {} command(s)", n);
    let cached = client
        .schema_cache()
        .get(__TELEPATH_CMD_PING.id)
        .expect("ping not present in SchemaCache after discover");
    assert_eq!(cached.name, "ping");
    assert!(
        !cached.args_schema.is_empty(),
        "ping args_schema must be populated by #[command]"
    );
    assert!(
        !cached.ret_schema.is_empty(),
        "ping ret_schema must be populated by #[command]"
    );

    // Signal the firmware thread to exit and wait for it.
    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");

    Ok(())
}
