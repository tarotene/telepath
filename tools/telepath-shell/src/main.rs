//! Telepath host-side CLI.
//!
//! Connects to a target running `telepath-server` and issues Telepath RPC calls.
//! Transport is selected at build time via Cargo features:
//!
//! - `--features rtt` (default): probe-rs RTT over J-Link / CMSIS-DAP.
//! - `--no-default-features --features serial`: CDC-ACM serial port (USB or physical UART).
//!
//! Exactly one transport feature must be enabled; enabling both or neither is a compile error.

#[cfg(all(feature = "rtt", feature = "serial"))]
compile_error!("telepath-shell: enable exactly one of `rtt`/`serial`; use --no-default-features --features serial for serial-only.");

#[cfg(not(any(feature = "rtt", feature = "serial")))]
compile_error!("telepath-shell: at least one transport feature must be enabled (`rtt` or `serial`).");

mod json_to_postcard;
mod postcard_to_json;

use anyhow::{bail, Context};
use clap::Parser;
use json_to_postcard::json_to_postcard;
use postcard_schema::schema::owned::{OwnedDataModelType, OwnedNamedType};
use postcard_to_json::postcard_to_json;
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context as RlContext, Editor, Helper};
use std::io::{self, Write};
#[cfg(feature = "rtt")]
use std::path::PathBuf;
use std::time::Duration;
#[cfg(feature = "rtt")]
use std::time::Instant;
use telepath_client::{HostTransportExt, TelepathClient};

#[cfg(feature = "rtt")]
use telepath_client::rtt_transport::RttTransport;
#[cfg(feature = "serial")]
use telepath_client::serial_transport::SerialTransport;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "telepath-shell", about = "Telepath RPC client")]
struct Cli {
    #[cfg(feature = "rtt")]
    /// Target chip name.
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    #[cfg(feature = "rtt")]
    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    #[cfg(feature = "rtt")]
    /// Destination for RTT channel 0 (firmware debug) logs.
    /// Use `-` for stderr, `/dev/null` to suppress.
    /// [default: $XDG_STATE_HOME/telepath/shell.log or ~/.local/state/telepath/shell.log]
    #[arg(long, value_name = "PATH")]
    log_file: Option<String>,

    #[cfg(feature = "rtt")]
    /// Disable automatic chip reset when the RTT control block is not found on attach.
    /// By default, telepath-shell issues a soft reset and retries the attach once.
    #[arg(long)]
    no_reset: bool,

    #[cfg(feature = "serial")]
    /// Serial port path (e.g. /dev/ttyUSB0, /dev/ttyACM0, COM3).
    #[arg(long)]
    port: String,

    #[cfg(feature = "serial")]
    /// Serial baud rate.
    #[arg(long, default_value = "115200")]
    baud: u32,

    /// Execute a single command non-interactively and exit.
    /// The argument uses the same syntax as the interactive REPL prompt:
    /// `--exec ping`, `--exec add 1 2`, `--exec led_set 1 true`, etc.
    /// Pass `--exec help [COMMAND]` to print help and exit.
    /// Exit code is non-zero if discovery or the command itself fails.
    #[arg(long, value_name = "COMMAND", num_args = 1..)]
    exec: Vec<String>,
}

#[cfg(feature = "rtt")]
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

    #[cfg(feature = "rtt")]
    {
        use probe_rs::{probe::list::Lister, Permissions};
        use std::fs::{self, File, OpenOptions};

        let mut log_sink: Box<dyn Write> = open_log_sink(cli.log_file.as_deref())?;

        let rtt_timing = std::env::var_os("TELEPATH_RTT_TIMING").is_some();
        let lister = Lister::new();
        let probes = lister.list_all();
        if probes.is_empty() {
            bail!("No debug probes found. Is the J-Link / CMSIS-DAP connected?");
        }
        let t_probe_open = Instant::now();
        let probe = probes
            .into_iter()
            .next()
            .unwrap()
            .open()
            .context("Failed to open debug probe")?;
        if rtt_timing {
            eprintln!(
                "[telepath:rtt-timing] probe.open elapsed={:?}",
                t_probe_open.elapsed()
            );
        }

        let t_session = Instant::now();
        let session = probe
            .attach(&cli.chip, Permissions::default())
            .with_context(|| format!("Failed to attach to target '{}'", cli.chip))?;
        if rtt_timing {
            eprintln!(
                "[telepath:rtt-timing] probe.attach({}) elapsed={:?}",
                cli.chip,
                t_session.elapsed()
            );
        }

        let transport =
            RttTransport::new(session, 0, 1, 1, cli.rtt_control_block_addr, !cli.no_reset)?;
        let mut client = TelepathClient::new(transport);

        run_session(&cli, &mut client, &mut *log_sink)?;

        // ----------- RTT-specific helpers (inner fns to limit scope) -----------

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
            if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
                return PathBuf::from(state_home).join("telepath").join("shell.log");
            }
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home)
                    .join(".local")
                    .join("state")
                    .join("telepath")
                    .join("shell.log");
            }
            eprintln!("Warning: $HOME not set, logging to ./telepath-shell.log");
            PathBuf::from("telepath-shell.log")
        }
    }

    #[cfg(feature = "serial")]
    {
        let transport = SerialTransport::new(&cli.port, cli.baud)?;
        let mut client = TelepathClient::new(transport);
        let mut log_sink: Box<dyn Write> = Box::new(io::sink());
        run_session(&cli, &mut client, &mut *log_sink)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Session logic (transport-agnostic)
// ---------------------------------------------------------------------------

fn run_session<T: HostTransportExt>(
    cli: &Cli,
    client: &mut TelepathClient<T>,
    log: &mut dyn Write,
) -> anyhow::Result<()> {
    client.transport_mut().drain_debug_logs(log);
    client.transport_mut().drain_rpc_rx();

    client
        .transport_mut()
        .set_read_deadline(Duration::from_secs(10));
    let n = client.discover().map_err(|e| {
        anyhow::anyhow!(
            "Command discovery failed ({e:?}) — is the firmware running and transport attached?"
        )
    })?;
    client.transport_mut().clear_read_deadline();

    if !cli.exec.is_empty() {
        let joined = cli.exec.join(" ");
        let line = joined.trim();
        if line.is_empty() {
            bail!("--exec requires a non-empty command");
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        if name == "help" {
            if rest.is_empty() {
                print_help(client);
            } else {
                print_command_help(client, rest);
            }
            return Ok(());
        }
        dispatch_command(client, name, rest)?;
        return Ok(());
    }

    println!("{n} command(s) discovered  (Ctrl-D / Ctrl-C to exit)");

    let mut commands: Vec<String> = client
        .schema_cache()
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    commands.push(String::from("help"));

    run_repl(client, log, commands)?;

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

fn run_repl<T: HostTransportExt>(
    client: &mut TelepathClient<T>,
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

fn elem_name(i: usize, raw: &str) -> String {
    if raw.is_empty() || raw.parse::<u64>().is_ok() {
        format!("arg{i}")
    } else {
        raw.to_string()
    }
}

fn print_help<T: HostTransportExt>(client: &TelepathClient<T>) {
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

fn print_command_help<T: HostTransportExt>(client: &TelepathClient<T>, cmd_name: &str) {
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

fn dispatch_command<T: HostTransportExt>(
    client: &mut TelepathClient<T>,
    name: &str,
    args_str: &str,
) -> anyhow::Result<()> {
    if args_str == "--help" || args_str == "-h" {
        print_command_help(client, name);
        return Ok(());
    }

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
