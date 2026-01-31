//! Top-level module wiring for tprosshy.
//!
//! This file ties together the major subsystems:
//! - Firewall manipulation (methods)
//! - SSH tunnel management
//! - Transparent proxying
//! - Logging and utilities
//!
//! It also defines the CLI interface via `clap`.

mod methods;
use methods::get_available_net_tool;

mod proxy;
pub use proxy::init_proxy;

mod ssh;
use ssh::get_remote_nameserver;
use ssh::ssh;

mod utils;
pub use utils::init_logger;
use utils::{get_local_nameserver, get_original_dst};

use clap::Parser;

/// Command-line arguments for tprosshy.
///
/// These options control how the SSH tunnel is established and how traffic
/// is redirected through it.
///
/// Argument parsing is handled by `clap` using derive macros.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// SSH config host.
    /// This corresponds to a host entry in `~/.ssh/config`, not a raw hostname.
    /// The value is passed directly to the `ssh` binary.
    #[arg(short = 'H', long)]
    pub host: String,

    /// Allowed IP range to be transparently proxied.
    /// This value is interpreted by the firewall backend (currently nftables).
    /// The default `0.0.0.0/0` redirects all IPv4 traffic.
    #[arg(long = "ip", default_value_t = String::from("0.0.0.0/0"))]
    pub ip_range: String,

    /// Local SOCKS5 port.
    /// This is the port used by `ssh -D` for dynamic port forwarding.
    /// Firewall rules will redirect TCP traffic to this port.
    #[arg(long, default_value_t = 1080)]
    pub socks_port: u16,

    /// Enable Tokio runtime tracing.
    /// When enabled, the program emits tracing data compatible with
    /// `tokio-console`. This requires building with
    /// `RUSTFLAGS="--cfg tokio_unstable"`.
    #[arg(long, default_value_t = false)]
    pub tracing: bool,
}
