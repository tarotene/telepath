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

use heapless::Vec as HVec;
use telepath_client::{HostError, TelepathClient};
use telepath_server::{command, TelepathServer};

mod loopback;

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    // CRC-32/ISO-HDLC: poly=0x04C11DB7, refin=true, refout=true,
    // init=0xFFFF_FFFF, xorout=0xFFFF_FFFF.
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

    // add
    let args_add = postcard::to_allocvec(&(2i32, 3i32)).expect("add args encode");
    let payload_add = client.call_raw(__TELEPATH_CMD_ADD.id, &args_add)?;
    let sum: i32 = postcard::from_bytes(&payload_add).expect("add returned invalid i32");
    assert_eq!(sum, 5, "add(2, 3) must return 5");
    println!("add -> {sum}");

    // crc32 over 128 zero bytes → 0xC2A8FA9D
    let mut zeros: HVec<u8, 128> = HVec::new();
    for _ in 0..128 {
        zeros.push(0).unwrap();
    }
    let args_crc = postcard::to_allocvec(&(zeros,)).expect("crc32 args encode");
    let payload_crc = client.call_raw(__TELEPATH_CMD_CRC32.id, &args_crc)?;
    let crc: u32 = postcard::from_bytes(&payload_crc).expect("crc32 returned invalid u32");
    assert_eq!(
        crc, 0xC2A8_FA9D,
        "crc32 over 128 zeros must equal 0xC2A8FA9D"
    );
    println!("crc32 -> 0x{crc:08X}");

    // echo
    let mut seq: HVec<u8, 128> = HVec::new();
    for i in 0u8..128 {
        seq.push(i).unwrap();
    }
    let args_echo = postcard::to_allocvec(&(seq.clone(),)).expect("echo args encode");
    let payload_echo = client.call_raw(__TELEPATH_CMD_ECHO.id, &args_echo)?;
    let echoed: HVec<u8, 128> =
        postcard::from_bytes(&payload_echo).expect("echo returned invalid payload");
    assert_eq!(echoed, seq, "echo must return input bytes unchanged");
    println!("echo -> ok");

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
