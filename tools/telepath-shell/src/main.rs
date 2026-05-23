//! Telepath host-side CLI.
//!
//! Connects to a target running `telepath-server` via a J-Link / CMSIS-DAP
//! probe using probe-rs, attaches to RTT, and issues Telepath RPC calls.
//!
//! RTT channel 0 (up) carries firmware debug prints and is forwarded to stderr.
//! RTT channel 1 (up/down) carries Telepath RPC traffic.
//!
//! # Modes
//!
//! **1-shot** — pass a subcommand:
//! ```
//! telepath-shell ping
//! telepath-shell --chip STM32F411RETx ping
//! ```
//!
//! **Interactive REPL** — no subcommand:
//! ```
//! telepath-shell
//! telepath-shell --chip nRF52840_xxAA
//! ```

mod rtt_transport;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use probe_rs::{probe::list::Lister, Permissions};
use rtt_transport::RttTransport;
use std::time::Duration;
use telepath_client::TelepathClient;
use telepath_wire::cmd_id::derive_cmd_id;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "telepath-shell", about = "Telepath RPC over RTT")]
struct Cli {
    /// Target chip name (default: nRF52840_xxAA).
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    /// Falls back to env var TELEPATH_RTT_CONTROL_BLOCK_ADDR, then 0x20000000.
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    #[command(subcommand)]
    command: Option<Command>,
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}

#[derive(Subcommand)]
enum Command {
    /// Send a ping and print the returned u32.
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
    let transport = RttTransport::new(session, 0, 1, 1, cli.rtt_control_block_addr)?;
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
    const CMD_PING: u16 = derive_cmd_id("ping", "()", "u32");

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
