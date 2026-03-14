//! Telepath host-side CLI.
//!
//! Connects to an nRF52840-DK (or any target running `telepath-firmware`) via
//! a J-Link / CMSIS-DAP probe using probe-rs, attaches to RTT channel 1, and
//! issues Telepath RPC calls.
//!
//! # Usage
//!
//! ```
//! telepath-cli ping
//! ```
//!
//! Expected output:
//! ```
//! ping -> 0xDEADBEEF
//! ```

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use probe_rs::{
    probe::list::Lister,
    rtt::Rtt,
    Permissions,
};
use telepath_wire::{
    framing::{cobs_decode, MAX_FRAME_SIZE},
    PacketType, Request, Response, ResponseStatus,
};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "telepath-cli", about = "Telepath RPC over RTT")]
struct Cli {
    /// Target chip name (default: nRF52840_xxAA).
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a ping (CmdID 0x0001) and print the returned u32.
    Ping,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Open the first available debug probe.
    let lister = Lister::new();
    let probes = lister.list_all();
    if probes.is_empty() {
        bail!("No debug probes found. Is the J-Link / CMSIS-DAP connected?");
    }
    let probe = probes
        .into_iter()
        .next()
        .unwrap()
        .open()
        .context("Failed to open debug probe")?;

    let mut session = probe
        .attach(&cli.chip, Permissions::default())
        .with_context(|| format!("Failed to attach to target '{}'", cli.chip))?;

    let mut core = session.core(0).context("Failed to access core 0")?;

    // Attach to the RTT control block in target RAM.
    let mut rtt = Rtt::attach(&mut core).context(
        "Failed to attach to RTT. Is the firmware running and RTT initialized?",
    )?;

    match cli.command {
        Command::Ping => cmd_ping(&mut rtt, &mut core)?,
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Send a ping request (CmdID 0x0001) and display the response.
fn cmd_ping(
    rtt: &mut Rtt,
    core: &mut probe_rs::Core<'_>,
) -> anyhow::Result<()> {
    const SEQ: u16 = 0;
    const CMD_PING: u16 = 0x0001;

    // Build and COBS-encode the request.
    let req = Request {
        kind: PacketType::Request,
        seq_no: SEQ,
        cmd_id: CMD_PING,
        args: &[],
    };
    let serialized = postcard::to_allocvec(&req).context("Failed to serialize ping request")?;
    let encoded_cap = cobs::max_encoding_length(serialized.len()) + 1;
    let mut encoded = vec![0u8; encoded_cap];
    let n = cobs::encode(&serialized, &mut encoded);
    encoded[n] = 0x00; // frame delimiter

    // Send via RTT down channel 1 (host→target).
    {
        let down = rtt
            .down_channel(1)
            .context("RTT down channel 1 not found — is the firmware compiled with Telepath?")?;
        down.write(core, &encoded[..n + 1])?;
    }

    // Read response bytes from RTT up channel 1 (target→host) until 0x00.
    let mut raw_frame: Vec<u8> = Vec::new();
    let mut buf = [0u8; 256];

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("Timed out waiting for ping response after 5 seconds");
        }

        let count = {
            let up = rtt
                .up_channel(1)
                .context("RTT up channel 1 not found")?;
            up.read(core, &mut buf)?
        };

        let mut frame_done = false;
        for &b in &buf[..count] {
            if b == 0x00 {
                frame_done = true;
                break;
            }
            raw_frame.push(b);
        }

        if frame_done {
            break;
        }

        if count == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    // COBS decode.
    let mut decoded = vec![0u8; MAX_FRAME_SIZE];
    let m = cobs_decode(&raw_frame, &mut decoded)
        .map_err(|e| anyhow::anyhow!("COBS decode failed: {:?}", e))?;

    // Deserialize Response.
    let resp: Response<'_> =
        postcard::from_bytes(&decoded[..m]).context("Failed to deserialize ping response")?;

    if resp.seq_no != SEQ {
        bail!(
            "Sequence number mismatch: expected {}, got {}",
            SEQ,
            resp.seq_no
        );
    }

    match resp.status {
        ResponseStatus::Ok => {
            let val: u32 =
                postcard::from_bytes(resp.payload).context("Failed to deserialize ping value")?;
            println!("ping -> 0x{:08X}", val);
            Ok(())
        }
        ResponseStatus::SystemError => bail!("Target returned SystemError"),
        ResponseStatus::AppError => bail!("Target returned AppError"),
    }
}
