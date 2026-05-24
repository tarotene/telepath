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
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context as RlContext, Editor, Helper};
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

    /// Disable automatic chip reset when the RTT control block is not found on attach.
    /// By default, telepath-shell issues a soft reset and retries the attach once.
    #[arg(long)]
    no_reset: bool,

    /// Execute a single command non-interactively and exit.
    /// The argument uses the same syntax as the interactive REPL prompt:
    /// `ping`, `add 1 2`, `led_set 1 true`, etc.
    /// Exit code is non-zero if discovery or the command itself fails.
    #[arg(long, value_name = "COMMAND")]
    exec: Option<String>,
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
    let transport = RttTransport::new(session, 0, 1, 1, cli.rtt_control_block_addr, !cli.no_reset)?;
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

    if let Some(line) = cli.exec.as_deref() {
        let line = line.trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        dispatch_command(&mut client, name, rest)?;
        return Ok(());
    }

    println!("{n} command(s) discovered  (Ctrl-D / Ctrl-C to exit)");

    let mut commands: Vec<String> = client
        .schema_cache()
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    commands.push(String::from("help"));

    run_repl(&mut client, &mut *log_sink, commands)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tab completion
// ---------------------------------------------------------------------------

struct CommandCompleter {
    commands: Vec<String>,
}

impl Completer for CommandCompleter {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &RlContext<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        // Only complete the first token (command name); ignore mid-line positions.
        if line[..pos].contains(char::is_whitespace) {
            return Ok((pos, vec![]));
        }
        let word = &line[..pos];
        let matches = self
            .commands
            .iter()
            .filter(|c| c.starts_with(word))
            .cloned()
            .collect();
        Ok((0, matches))
    }
}

impl Helper for CommandCompleter {}
impl Hinter for CommandCompleter {
    type Hint = String;
}
impl Highlighter for CommandCompleter {}
impl Validator for CommandCompleter {}

// ---------------------------------------------------------------------------
// Interactive REPL
// ---------------------------------------------------------------------------

fn run_repl(
    client: &mut TelepathClient<RttTransport>,
    log: &mut dyn Write,
    commands: Vec<String>,
) -> anyhow::Result<()> {
    let mut rl = Editor::<CommandCompleter, DefaultHistory>::new()?;
    rl.set_helper(Some(CommandCompleter { commands }));

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
                    "help" => {
                        if rest.is_empty() {
                            print_help(client);
                        } else {
                            print_command_help(client, rest);
                        }
                    }
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

fn type_label(ty: &OwnedDataModelType) -> &'static str {
    match ty {
        OwnedDataModelType::Bool => "bool",
        OwnedDataModelType::I8 => "i8",
        OwnedDataModelType::U8 => "u8",
        OwnedDataModelType::I16 => "i16",
        OwnedDataModelType::U16 => "u16",
        OwnedDataModelType::I32 => "i32",
        OwnedDataModelType::U32 => "u32",
        OwnedDataModelType::I64 => "i64",
        OwnedDataModelType::U64 => "u64",
        OwnedDataModelType::F32 => "f32",
        OwnedDataModelType::F64 => "f64",
        OwnedDataModelType::Char => "char",
        OwnedDataModelType::String => "str",
        OwnedDataModelType::ByteArray => "bytes",
        OwnedDataModelType::Option(_) => "option",
        OwnedDataModelType::Unit | OwnedDataModelType::UnitStruct => "()",
        OwnedDataModelType::Seq(_) => "array",
        _ => "?",
    }
}

fn scalar_example(ty: &OwnedDataModelType) -> &'static str {
    match ty {
        OwnedDataModelType::Bool => "false",
        OwnedDataModelType::I8
        | OwnedDataModelType::U8
        | OwnedDataModelType::I16
        | OwnedDataModelType::U16
        | OwnedDataModelType::I32
        | OwnedDataModelType::U32
        | OwnedDataModelType::I64
        | OwnedDataModelType::U64 => "0",
        OwnedDataModelType::F32 | OwnedDataModelType::F64 => "0.0",
        OwnedDataModelType::Char => "\"a\"",
        OwnedDataModelType::String => "\"hello\"",
        OwnedDataModelType::Option(_) => "null",
        OwnedDataModelType::Seq(_) | OwnedDataModelType::ByteArray => "[]",
        _ => "0",
    }
}

/// Build a POSIX-style `<name: type>` argument string for top-level help.
fn args_display(schema: &OwnedNamedType) -> String {
    let elems = match &schema.ty {
        OwnedDataModelType::Unit | OwnedDataModelType::UnitStruct => return String::new(),
        OwnedDataModelType::Tuple(elems) => elems,
        _ => return format!("<{}>", schema.name),
    };
    elems
        .iter()
        .enumerate()
        .map(|(i, elem)| {
            let name = elem_name(i, &elem.name);
            format!("<{}: {}>", name, type_label(&elem.ty))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return a display name for a tuple element: fall back to `arg{i}` for numeric/empty names.
fn elem_name(i: usize, raw: &str) -> String {
    if raw.is_empty() || raw.parse::<u64>().is_ok() {
        format!("arg{i}")
    } else {
        raw.to_string()
    }
}

fn print_help(client: &TelepathClient<RttTransport>) {
    let mut entries: Vec<_> = client.schema_cache().iter().collect();
    entries.sort_by_key(|e| e.name.as_str());

    let rows: Vec<(String, String)> = entries
        .iter()
        .map(|entry| {
            let args = entry
                .decoded_args_schema()
                .map(|t| args_display(&t))
                .unwrap_or_default();
            let usage = if args.is_empty() {
                entry.name.to_string()
            } else {
                format!("{} {}", entry.name, args)
            };
            let ret = entry
                .decoded_ret_schema()
                .map(|t| type_label(&t.ty).to_string())
                .unwrap_or_else(|_| "?".into());
            (usage, ret)
        })
        .collect();

    let col_width = rows.iter().map(|(u, _)| u.len()).max().unwrap_or(0).max(24);

    println!("Commands:");
    for (usage, ret) in &rows {
        println!("  {usage:<col_width$}  -> {ret}");
    }
    println!();
    println!(
        "  {:<col_width$}  Show this help or detail for a command",
        "help [COMMAND]"
    );
}

fn print_command_help(client: &TelepathClient<RttTransport>, cmd_name: &str) {
    let cache = client.schema_cache();
    let Some(entry) = cache.iter().find(|e| e.name == cmd_name) else {
        eprintln!("Unknown command: {cmd_name}  (try 'help')");
        return;
    };

    let Ok(args_schema) = entry.decoded_args_schema() else {
        eprintln!("Could not decode args schema for '{cmd_name}'");
        return;
    };
    let Ok(ret_schema) = entry.decoded_ret_schema() else {
        eprintln!("Could not decode ret schema for '{cmd_name}'");
        return;
    };

    let args_disp = args_display(&args_schema);
    let ret_lbl = type_label(&ret_schema.ty);

    if args_disp.is_empty() {
        println!("{cmd_name} -> {ret_lbl}");
    } else {
        println!("{cmd_name} {args_disp} -> {ret_lbl}");
    }

    if let OwnedDataModelType::Tuple(elems) = &args_schema.ty {
        if !elems.is_empty() {
            println!();
            println!("  Arguments:");
            let name_width = elems
                .iter()
                .enumerate()
                .map(|(i, e)| elem_name(i, &e.name).len())
                .max()
                .unwrap_or(0);

            for (i, elem) in elems.iter().enumerate() {
                let name = elem_name(i, &elem.name);
                println!(
                    "    <{:<name_width$}>  {:<6}  Example: {}",
                    name,
                    type_label(&elem.ty),
                    scalar_example(&elem.ty)
                );
            }

            let examples: Vec<&str> = elems.iter().map(|e| scalar_example(&e.ty)).collect();
            println!();
            println!("  Returns: {ret_lbl}");
            println!("  Usage:   {cmd_name} [{}]", examples.join(", "));
            return;
        }
    }

    println!();
    println!("  Returns: {ret_lbl}");
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn dispatch_command(
    client: &mut TelepathClient<RttTransport>,
    name: &str,
    args_str: &str,
) -> anyhow::Result<()> {
    if args_str == "--help" || args_str == "-h" {
        print_command_help(client, name);
        return Ok(());
    }

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

    format_result(name, &ret_schema, result);
    Ok(())
}

fn format_result(name: &str, ret_schema: &OwnedNamedType, val: serde_json::Value) {
    match &ret_schema.ty {
        OwnedDataModelType::U8 => println!("{name} -> 0x{:02X}", val.as_u64().unwrap_or(0)),
        OwnedDataModelType::U16 => println!("{name} -> 0x{:04X}", val.as_u64().unwrap_or(0)),
        OwnedDataModelType::U32 => println!("{name} -> 0x{:08X}", val.as_u64().unwrap_or(0)),
        OwnedDataModelType::U64 => println!("{name} -> 0x{:016X}", val.as_u64().unwrap_or(0)),
        OwnedDataModelType::Unit | OwnedDataModelType::UnitStruct => println!("{name} OK"),
        _ => println!("{name} -> {val}"),
    }
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
            "'{cmd_name}' expects arguments ({}).  Pass them as a JSON array, e.g.: telepath> {cmd_name} [<arg1>, <arg2>, ...]",
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
        Some("/dev/null") => Ok(Box::new(io::sink())),
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
