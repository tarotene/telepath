//! Telepath host-side CLI.
//!
//! Connects to a target running `telepath-firmware` via a J-Link / CMSIS-DAP
//! probe using probe-rs, attaches to RTT, and issues Telepath RPC calls.
//!
//! RTT channel 0 (up) carries firmware debug prints and is forwarded to stderr.
//! RTT channel 1 (up/down) carries Telepath RPC traffic.
//!
//! # Modes
//!
//! **1-shot** — pass a subcommand:
//! ```
//! telepath-cli ping
//! telepath-cli --chip STM32F411RETx ping
//! ```
//!
//! **Interactive REPL** — no subcommand:
//! ```
//! telepath-cli
//! telepath-cli --chip nRF52840_xxAA
//! ```

mod rtt_transport;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use probe_rs::{probe::list::Lister, Permissions};
use rtt_transport::RttTransport;
use std::time::Duration;
use telepath_host::TelepathClient;

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
    command: Option<Command>,
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

    let session = probe
        .attach(&cli.chip, Permissions::default())
        .with_context(|| format!("Failed to attach to target '{}'", cli.chip))?;

    // RTT channels: up 1 / down 1 for RPC, up 0 for debug logs.
    let transport = RttTransport::new(session, 0, 1, 1)?;
    let mut client = TelepathClient::new(transport);

    match cli.command {
        Some(Command::Ping) => {
            client.transport_mut().drain_debug_logs();
            cmd_ping(&mut client)?;
        }
        None => {
            run_repl(&mut client)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive REPL
// ---------------------------------------------------------------------------

fn run_repl(client: &mut TelepathClient<RttTransport>) -> anyhow::Result<()> {
    let mut rl = rustyline::DefaultEditor::new()?;
    loop {
        // Forward any pending debug output before showing the prompt.
        client.transport_mut().drain_debug_logs();

        match rl.readline("telepath> ") {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);
                match line.as_str() {
                    "ping" => cmd_ping(client)?,
                    "help" => println!("Commands: ping, help, quit"),
                    "quit" | "exit" => break,
                    other => eprintln!("Unknown command: {other}  (try 'help')"),
                }
            }
            Err(
                rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof,
            ) => break,
            Err(e) => bail!(e),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_ping(client: &mut TelepathClient<RttTransport>) -> anyhow::Result<()> {
    const CMD_PING: u16 = 0x0001;

    client
        .transport_mut()
        .set_read_deadline(Duration::from_secs(5));

    let payload = client
        .call_raw(CMD_PING, &[])
        .map_err(|e| anyhow::anyhow!("ping failed: {:?}", e))?;

    let val: u32 = postcard::from_bytes(&payload).context("Failed to deserialize ping value")?;
    println!("ping -> 0x{:08X}", val);
    Ok(())
}
