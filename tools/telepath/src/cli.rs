use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "telepath", about = "Unified CLI for Telepath RPC")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Interactive shell for Telepath RPC commands
    #[cfg(feature = "shell")]
    Shell(ShellArgs),

    /// MCP server exposing Telepath commands as MCP tools
    #[cfg(feature = "mcp")]
    Mcp(McpArgs),
}

#[cfg(feature = "shell")]
#[derive(Args)]
pub struct ShellArgs {
    #[command(flatten)]
    pub transport: TransportArgs,

    /// Destination for RTT channel 0 (firmware debug) logs.
    /// Use `-` for stderr, `/dev/null` to suppress.
    /// [default: $XDG_STATE_HOME/telepath/shell.log or ~/.local/state/telepath/shell.log]
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<String>,

    /// Execute a single command non-interactively and exit.
    /// The argument uses the same syntax as the interactive REPL prompt:
    /// `--exec ping`, `--exec add 1 2`, `--exec led_set 1 true`, etc.
    /// Pass `--exec help [COMMAND]` to print help and exit.
    /// Exit code is non-zero if discovery or the command itself fails.
    #[arg(long, value_name = "COMMAND", num_args = 1..)]
    pub exec: Vec<String>,
}

#[cfg(feature = "mcp")]
#[derive(Args)]
pub struct McpArgs {
    #[command(flatten)]
    pub transport: TransportArgs,
}

#[derive(Args)]
pub struct TransportArgs {
    /// Transport backend to use for communicating with the target.
    #[arg(long, value_enum, default_value = "rtt")]
    pub transport: TransportKind,

    /// Target chip name (RTT transport only).
    #[arg(long, default_value = "nRF52840_xxAA")]
    pub chip: String,

    /// SEGGER RTT control block address in hex (RTT transport only).
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    pub rtt_control_block_addr: u64,

    /// Disable automatic chip reset when the RTT control block is not found on attach.
    #[arg(long)]
    pub no_reset: bool,

    /// Serial port path (serial transport only, e.g. /dev/ttyUSB0, /dev/ttyACM0, COM3).
    #[arg(long)]
    pub port: Option<String>,

    /// Serial baud rate (serial transport only).
    #[arg(long, default_value = "115200")]
    pub baud: u32,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum TransportKind {
    /// probe-rs RTT over J-Link / CMSIS-DAP
    Rtt,
    /// CDC-ACM serial port (USB or physical UART)
    Serial,
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}
