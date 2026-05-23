//! Telepath host-side CLI.
//!
//! Connects to a target running `telepath-server` via a J-Link / CMSIS-DAP
//! probe using probe-rs, attaches to RTT, and issues Telepath RPC calls.
//!
//! RTT channel 0 (up) carries firmware debug prints and is forwarded to the
//! log file (default: $XDG_STATE_HOME/telepath/shell.log).
//! RTT channel 1 (up/down) carries Telepath RPC traffic.
//!
//! # Usage
//!
//! ```
//! telepath-shell
//! telepath-shell --chip nRF52840_xxAA
//! telepath-shell --log-file /dev/null
//! ```

mod json_to_postcard;
mod postcard_to_json;
mod rtt_transport;

use anyhow::{bail, Context};
use clap::Parser;
use json_to_postcard::json_to_postcard;
use postcard_schema::schema::owned::{OwnedDataModelType, OwnedNamedType};
use postcard_to_json::postcard_to_json;
use probe_rs::{probe::list::Lister, Permissions};
use rtt_transport::RttTransport;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use telepath_client::TelepathClient;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "telepath-shell", about = "Telepath RPC over RTT")]
struct Cli {
    /// Target chip name.
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    /// Destination for RTT channel 0 (firmware debug) logs.
    /// Use `-` for stderr, `/dev/null` to suppress.
    /// [default: $XDG_STATE_HOME/telepath/shell.log or ~/.local/state/telepath/shell.log]
    #[arg(long, value_name = "PATH")]
    log_file: Option<String>,
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Resolve log sink and print startup banner.
    let mut log_sink: Box<dyn Write> = open_log_sink(cli.log_file.as_deref())?;

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

    // Drain any startup messages from the firmware before discovery.
    client.transport_mut().drain_debug_logs(&mut *log_sink);

    // Discover all commands exposed by the firmware.
    client
        .transport_mut()
        .set_read_deadline(Duration::from_secs(10));
    let n = client.discover().map_err(|e| {
        anyhow::anyhow!(
            "Command discovery failed ({e:?}) — is the firmware running and RTT attached?"
        )
    })?;
    client.transport_mut().clear_read_deadline();
    println!("{n} command(s) discovered");

    run_repl(&mut client, &mut *log_sink)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive REPL
// ---------------------------------------------------------------------------

fn run_repl(client: &mut TelepathClient<RttTransport>, log: &mut dyn Write) -> anyhow::Result<()> {
    let mut rl = rustyline::DefaultEditor::new()?;
    loop {
        client.transport_mut().drain_debug_logs(log);

        match rl.readline("telepath> ") {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);

                let mut parts = line.splitn(2, char::is_whitespace);
                let cmd_name = parts.next().unwrap_or("");
                let rest = parts.next().unwrap_or("").trim();

                match cmd_name {
                    "help" => print_help(client),
                    "quit" | "exit" => break,
                    name => {
                        if let Err(e) = dispatch_command(client, name, rest) {
                            eprintln!("Error: {e}");
                        }
                    }
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
// Help
// ---------------------------------------------------------------------------

fn print_help(client: &TelepathClient<RttTransport>) {
    let mut entries: Vec<_> = client.schema_cache().iter().collect();
    entries.sort_by_key(|e| e.name.as_str());
    println!("Available commands:");
    for entry in entries {
        let args_label = entry
            .decoded_args_schema()
            .map(|t| t.name.to_string())
            .unwrap_or_else(|_| "?".into());
        let ret_label = entry
            .decoded_ret_schema()
            .map(|t| t.name.to_string())
            .unwrap_or_else(|_| "?".into());
        println!("  {} {} -> {}", entry.name, args_label, ret_label);
    }
    println!("  help");
    println!("  quit");
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn dispatch_command(
    client: &mut TelepathClient<RttTransport>,
    name: &str,
    args_str: &str,
) -> anyhow::Result<()> {
    // Extract what we need from the cache before the mutable borrow for call_raw.
    let (cmd_id, args_schema, ret_schema) = {
        let cache = client.schema_cache();
        let entry = cache
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}  (try 'help')"))?;
        let args = entry
            .decoded_args_schema()
            .map_err(|_| anyhow::anyhow!("Failed to decode args schema for '{name}'"))?;
        let ret = entry
            .decoded_ret_schema()
            .map_err(|_| anyhow::anyhow!("Failed to decode ret schema for '{name}'"))?;
        (entry.cmd_id, args, ret)
    };

    let args_bytes = encode_args(&args_schema, args_str, name)?;

    client
        .transport_mut()
        .set_read_deadline(Duration::from_secs(5));

    let response = client
        .call_raw(cmd_id, &args_bytes)
        .map_err(|e| anyhow::anyhow!("'{name}' call failed: {e:?}"))?;

    let result = postcard_to_json(&ret_schema, &response)
        .map_err(|e| anyhow::anyhow!("Response decoding failed: {e}"))?;

    println!("{name} -> {result}");
    Ok(())
}

/// Encode CLI argument string into postcard bytes according to the args schema.
///
/// 0-arg functions (`Unit`) accept no arguments.
/// N-arg functions expect a JSON array, e.g. `[1, true]`.
fn encode_args(
    args_schema: &OwnedNamedType,
    args_str: &str,
    cmd_name: &str,
) -> anyhow::Result<Vec<u8>> {
    let is_unit = matches!(
        &args_schema.ty,
        OwnedDataModelType::Unit | OwnedDataModelType::UnitStruct
    );

    if is_unit {
        if !args_str.is_empty() {
            bail!("'{cmd_name}' takes no arguments, but got: {args_str}");
        }
        return Ok(vec![]);
    }

    let json_val: serde_json::Value = if args_str.is_empty() {
        bail!(
            "'{cmd_name}' expects arguments ({}).  Pass them as a JSON array, e.g.: [{cmd_name}] [<arg1>, <arg2>, ...]",
            args_schema.name
        );
    } else {
        serde_json::from_str(args_str)
            .with_context(|| format!("Invalid JSON arguments for '{cmd_name}': {args_str}"))?
    };

    json_to_postcard(args_schema, &json_val)
        .map_err(|e| anyhow::anyhow!("Argument encoding failed for '{cmd_name}': {e}"))
}

// ---------------------------------------------------------------------------
// Log sink
// ---------------------------------------------------------------------------

/// Open the RTT channel 0 log sink.
///
/// Special values for `spec`:
/// - `None`: use XDG default path ($XDG_STATE_HOME/telepath/shell.log)
/// - `Some("-")`: stderr
/// - `Some("/dev/null")`: discard (writes to /dev/null)
/// - `Some(path)`: append to the given file
fn open_log_sink(spec: Option<&str>) -> anyhow::Result<Box<dyn Write>> {
    match spec {
        Some("-") => {
            println!("Firmware RTT ch0 logs -> stderr (may interleave with prompt)");
            Ok(Box::new(io::stderr()))
        }
        Some(path) => {
            let path = PathBuf::from(path);
            let (file, label) = open_log_file(&path)?;
            println!("Firmware RTT ch0 logs -> {} ({})", path.display(), label);
            println!(
                "Tip: run `tail -F {}` in another terminal to follow.",
                path.display()
            );
            Ok(Box::new(file))
        }
        None => {
            let path = default_log_path();
            let (file, label) = open_log_file(&path)?;
            println!("Firmware RTT ch0 logs -> {} ({})", path.display(), label);
            println!(
                "Tip: run `tail -F {}` in another terminal to follow.",
                path.display()
            );
            Ok(Box::new(file))
        }
    }
}

fn open_log_file(path: &PathBuf) -> anyhow::Result<(File, &'static str)> {
    let label = if path.exists() { "append" } else { "new" };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create log directory '{}'", parent.display())
            })?;
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open log file '{}'", path.display()))?;
    Ok((file, label))
}

fn default_log_path() -> PathBuf {
    // XDG_STATE_HOME / telepath / shell.log
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("telepath").join("shell.log");
    }
    // ~/.local/state/telepath/shell.log
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("telepath")
            .join("shell.log");
    }
    // Last resort: ./telepath-shell.log
    eprintln!("Warning: $HOME not set, logging to ./telepath-shell.log");
    PathBuf::from("telepath-shell.log")
}
