mod methods;
pub use methods::get_available_net_tool;

mod proxy;
pub use proxy::init_proxy;

mod ssh;
pub use ssh::ssh;

pub mod utils;
pub use utils::{get_original_dst, init_logger};

use clap::Parser;

/// Transparent proxy over ssh. Local proxy.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// SSH config host.
    #[arg(short = 'H', long)]
    pub host: String,

    /// Allowed IP range
    #[arg(long = "ip", default_value_t = String::from("0.0.0.0/0"))]
    pub ip_range: String,

    /// Enable dns proxy
    #[arg(short, long, default_value_t = false)]
    pub dns: bool,

    /// Enable dns proxy
    #[arg(long, default_value_t = 1080)]
    pub dynamic_port: u16,
}
