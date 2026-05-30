use anyhow::bail;
#[cfg(feature = "rtt")]
use anyhow::Context;
use postcard_schema::schema::owned::{OwnedDataModelType, OwnedNamedType};
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
use telepath_client::{HostTransportExt, NameResolutionResult, TelepathClient};

use super::super::cli::ShellArgs;
use super::super::transport::AnyTransport;
use telepath::bridge;

pub fn run(args: &ShellArgs, mut client: TelepathClient<AnyTransport>) -> anyhow::Result<()> {
    let mut log_sink: Box<dyn Write> = open_log_sink(args.log_file.as_deref(), &mut client)?;

    client.transport_mut().drain_debug_logs(&mut *log_sink);
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

    #[cfg(feature = "profile")]
    if let Some(ref duration_str) = args.ping_storm {
        run_ping_storm(&mut client, duration_str)?;
        return Ok(());
    }

    if !args.exec.is_empty() {
        let joined = args.exec.join(" ");
        let line = joined.trim();
        if line.is_empty() {
            bail!("--exec requires a non-empty command");
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        if name == "help" {
            if rest.is_empty() {
                print_help(&client);
            } else {
                print_command_help(&client, rest);
            }
            return Ok(());
        }
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

fn open_log_sink(
    spec: Option<&str>,
    client: &mut TelepathClient<AnyTransport>,
) -> anyhow::Result<Box<dyn Write>> {
    #[cfg(feature = "rtt")]
    if matches!(client.transport_mut(), AnyTransport::Rtt(_)) {
        return open_rtt_log_sink(spec);
    }

    let _ = (spec, client);
    Ok(Box::new(io::sink()))
}

#[cfg(feature = "rtt")]
fn open_rtt_log_sink(spec: Option<&str>) -> anyhow::Result<Box<dyn Write>> {
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

#[cfg(feature = "rtt")]
fn open_log_file(path: &PathBuf) -> anyhow::Result<(std::fs::File, &'static str)> {
    use std::fs::{self, OpenOptions};

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

#[cfg(feature = "rtt")]
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
    eprintln!("Warning: $HOME not set, logging to ./telepath.log");
    PathBuf::from("telepath.log")
}

// ── Tab completion ──────────────────────────────────────────────────────

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

// ── Interactive REPL ────────────────────────────────────────────────────

fn run_repl(
    client: &mut TelepathClient<AnyTransport>,
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

// ── Help ────────────────────────────────────────────────────────────────

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

fn print_help(client: &TelepathClient<AnyTransport>) {
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

fn print_command_help(client: &TelepathClient<AnyTransport>, cmd_name: &str) {
    let cmd_id = match client.cmd_id_by_name(cmd_name) {
        NameResolutionResult::Unique(id) => id,
        NameResolutionResult::NotFound => {
            eprintln!("Unknown command: {cmd_name}  (try 'help')");
            return;
        }
        NameResolutionResult::Ambiguous(ids) => {
            eprintln!(
                "Ambiguous command: '{cmd_name}' matches cmd_ids {:04X?}",
                ids
            );
            return;
        }
    };
    let Some(entry) = client.schema_cache().get(cmd_id) else {
        eprintln!("Schema not found for '{cmd_name}'");
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

// ── Command dispatch ────────────────────────────────────────────────────

fn dispatch_command(
    client: &mut TelepathClient<AnyTransport>,
    name: &str,
    args_str: &str,
) -> anyhow::Result<()> {
    if args_str == "--help" || args_str == "-h" {
        print_command_help(client, name);
        return Ok(());
    }

    let (cmd_id, args_schema, ret_schema) = {
        let resolved_id = match client.cmd_id_by_name(name) {
            NameResolutionResult::Unique(id) => id,
            NameResolutionResult::NotFound => {
                return Err(anyhow::anyhow!("Unknown command: {name}  (try 'help')"));
            }
            NameResolutionResult::Ambiguous(ids) => {
                return Err(anyhow::anyhow!(
                    "Ambiguous command: '{name}' matches multiple cmd_ids {:04X?}. \
                     Rename or remove the duplicate command(s) in firmware so each \
                     name is unique.",
                    ids
                ));
            }
        };
        let cache = client.schema_cache();
        let entry = cache
            .get(resolved_id)
            .ok_or_else(|| anyhow::anyhow!("Schema not found for '{name}'"))?;
        let args = entry
            .decoded_args_schema()
            .map_err(|_| anyhow::anyhow!("Failed to decode args schema for '{name}'"))?;
        let ret = entry
            .decoded_ret_schema()
            .map_err(|_| anyhow::anyhow!("Failed to decode ret schema for '{name}'"))?;
        (resolved_id, args, ret)
    };

    let args_json = encode_args(&args_schema, args_str, name)?;

    client
        .transport_mut()
        .set_read_deadline(Duration::from_secs(5));

    let result = bridge::invoke(client, cmd_id, &args_schema, &ret_schema, &args_json)
        .map_err(|e| anyhow::anyhow!("'{name}' call failed: {e}"))?;

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
) -> anyhow::Result<serde_json::Value> {
    let is_unit = matches!(
        &args_schema.ty,
        OwnedDataModelType::Unit | OwnedDataModelType::UnitStruct
    );

    if is_unit {
        if !args_str.is_empty() {
            bail!("'{cmd_name}' takes no arguments, but got: {args_str}");
        }
        return Ok(serde_json::Value::Null);
    }

    if args_str.is_empty() {
        bail!(
            "'{cmd_name}' expects arguments ({}). \
             Each positional arg is parsed as JSON, e.g.: {cmd_name} <arg1> <arg2> ...  \
             (JSON array form also supported: {cmd_name} [<arg1>, <arg2>, ...])",
            args_schema.name
        );
    }

    let json_val: serde_json::Value = match serde_json::from_str(args_str) {
        Ok(v) => v,
        Err(_) => {
            let tokens: Result<Vec<serde_json::Value>, _> = args_str
                .split_whitespace()
                .map(serde_json::from_str)
                .collect();
            match tokens {
                Ok(vals) => serde_json::Value::Array(vals),
                Err(e) => bail!(
                    "Invalid arguments for '{cmd_name}': {e}. \
                     Note: positional args are split on whitespace; \
                     for JSON strings or objects containing spaces, use the array form: \
                     {cmd_name} [<arg1>, <arg2>, ...]"
                ),
            }
        }
    };

    Ok(json_val)
}

// ── Ping-storm benchmark (profile feature) ──────────────────────────────

#[cfg(feature = "profile")]
fn parse_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    if let Some(secs) = s.strip_suffix('s') {
        let v: f64 = secs
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration '{s}': expected e.g. '10s'"))?;
        return Ok(std::time::Duration::from_secs_f64(v));
    }
    anyhow::bail!("invalid duration '{s}': only seconds suffix supported (e.g. '10s')")
}

#[cfg(feature = "profile")]
fn run_ping_storm(
    client: &mut TelepathClient<AnyTransport>,
    duration_str: &str,
) -> anyhow::Result<()> {
    use telepath_client::HostMetricsSnapshot;
    use telepath_wire::CMD_ID_METRICS;

    let dur = parse_duration(duration_str)?;

    let ping_id = match client.cmd_id_by_name("ping") {
        NameResolutionResult::Unique(id) => id,
        NameResolutionResult::NotFound => {
            return Err(anyhow::anyhow!("'ping' command not found in firmware"));
        }
        NameResolutionResult::Ambiguous(ids) => {
            return Err(anyhow::anyhow!(
                "Ambiguous 'ping' command: matches cmd_ids {:04X?}",
                ids
            ));
        }
    };

    // Drain any stale metrics from the target before the run.
    let _ = client.call_raw(CMD_ID_METRICS, &[]);
    // Reset host-side counters.
    let _ = client.take_host_metrics();

    let empty_args = postcard::to_allocvec(&())?;

    client
        .transport_mut()
        .set_read_deadline(std::time::Duration::from_secs(5));

    let start = std::time::Instant::now();
    let mut frames: u64 = 0;
    while start.elapsed() < dur {
        client
            .call_raw(ping_id, &empty_args)
            .map_err(|e| anyhow::anyhow!("ping call failed: {e:?}"))?;
        frames += 1;
    }
    let elapsed = start.elapsed();

    let host: HostMetricsSnapshot = client.take_host_metrics();

    // Attempt to retrieve target-side DWT metrics; degrade gracefully if not available.
    let target = client
        .call_raw(CMD_ID_METRICS, &[])
        .ok()
        .and_then(|raw| postcard::from_bytes::<telepath_wire::MetricsSnapshot>(&raw).ok());

    let elapsed_secs = elapsed.as_secs_f64();
    let fps = frames as f64 / elapsed_secs;

    eprintln!("ping-storm {elapsed_secs:.1}s: {frames} frames  ({fps:.1} frames/sec)");
    eprintln!("  downstream (host encode → target decode):");
    if let Some(ref t) = target {
        if t.sample_count > 0 && t.decoded_bytes > 0 {
            let cyc_per_byte = t.decode_cycles as f64 / t.decoded_bytes as f64;
            eprintln!(
                "    target: {cyc_per_byte:.1} cycles/byte  (sample={})",
                t.sample_count
            );
        } else {
            eprintln!("    target: n/a (no samples)");
        }
    } else {
        eprintln!("    target: n/a (get_metrics not available)");
    }
    if host.sample_count > 0 && host.encoded_bytes > 0 {
        let ns_per_byte = host.encode_ns as f64 / host.encoded_bytes as f64;
        eprintln!(
            "    host  : {ns_per_byte:.1} ns/byte  (sample={})",
            host.sample_count
        );
    } else {
        eprintln!("    host  : n/a (no samples)");
    }
    eprintln!("  upstream (target encode → host decode):");
    if let Some(ref t) = target {
        if t.sample_count > 0 && t.encoded_bytes > 0 {
            let cyc_per_byte = t.encode_cycles as f64 / t.encoded_bytes as f64;
            eprintln!(
                "    target: {cyc_per_byte:.1} cycles/byte  (sample={})",
                t.sample_count
            );
        } else {
            eprintln!("    target: n/a (no samples)");
        }
    } else {
        eprintln!("    target: n/a (get_metrics not available)");
    }
    if host.sample_count > 0 && host.decoded_bytes > 0 {
        let ns_per_byte = host.decode_ns as f64 / host.decoded_bytes as f64;
        eprintln!(
            "    host  : {ns_per_byte:.1} ns/byte  (sample={})",
            host.sample_count
        );
    } else {
        eprintln!("    host  : n/a (no samples)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use postcard_schema::schema::owned::{OwnedDataModelType as DMT, OwnedNamedType};

    fn wrap(name: &str, ty: DMT) -> OwnedNamedType {
        OwnedNamedType {
            name: name.to_string(),
            ty,
        }
    }

    #[test]
    fn encode_args_single_bare_scalar() {
        let schema = wrap("args", DMT::Tuple(vec![wrap("mask", DMT::U8)]));
        let result = encode_args(&schema, "10", "led_pattern").unwrap();
        // Bare "10" parses as JSON Number(10); the codec's 1-element tuple
        // unwrap accepts this without array wrapping.
        assert_eq!(result, serde_json::json!(10));
    }

    #[test]
    fn encode_args_single_json_array_backward_compat() {
        let schema = wrap("args", DMT::Tuple(vec![wrap("mask", DMT::U8)]));
        let result = encode_args(&schema, "[10]", "led_pattern").unwrap();
        assert_eq!(result, serde_json::json!([10]));
    }

    #[test]
    fn encode_args_multi_positional() {
        let schema = wrap(
            "args",
            DMT::Tuple(vec![wrap("a", DMT::I32), wrap("b", DMT::I32)]),
        );
        let positional = encode_args(&schema, "2 3", "add").unwrap();
        let array = encode_args(&schema, "[2, 3]", "add").unwrap();
        assert_eq!(positional, array);
    }

    #[test]
    fn encode_args_unit_rejects_args() {
        let schema = wrap("args", DMT::Unit);
        assert!(encode_args(&schema, "10", "ping").is_err());
    }

    #[test]
    fn encode_args_empty_string_is_error() {
        let schema = wrap("args", DMT::Tuple(vec![wrap("a", DMT::U8)]));
        assert!(encode_args(&schema, "", "cmd").is_err());
    }

    #[test]
    fn encode_args_negative_numbers() {
        let schema = wrap(
            "args",
            DMT::Tuple(vec![wrap("a", DMT::I32), wrap("b", DMT::I32)]),
        );
        let positional = encode_args(&schema, "-2 3", "add").unwrap();
        let array = encode_args(&schema, "[-2, 3]", "add").unwrap();
        assert_eq!(positional, array);
    }

    #[test]
    fn encode_args_empty_string_error_mentions_json_array_form() {
        let schema = wrap("args", DMT::Tuple(vec![wrap("a", DMT::U8)]));
        let err = encode_args(&schema, "", "cmd").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("JSON"),
            "error should mention JSON parsing: {msg}"
        );
        assert!(
            msg.contains("[<arg1>"),
            "error should hint at array form: {msg}"
        );
    }

    #[test]
    fn encode_args_invalid_token_error_mentions_whitespace_limitation() {
        let schema = wrap("args", DMT::Tuple(vec![wrap("a", DMT::U8)]));
        let err = encode_args(&schema, "foo bar", "cmd").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("whitespace"),
            "error should mention whitespace tokenisation: {msg}"
        );
        assert!(
            msg.contains("array form"),
            "error should hint at array form fallback: {msg}"
        );
    }
}
