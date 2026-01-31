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

// Transparent proxy over ssh. Local proxy.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    // SSH config host.
    #[arg(short = 'H', long)]
    pub host: String,

    // Allowed IP range
    #[arg(long = "ip", default_value_t = String::from("0.0.0.0/0"))]
    pub ip_range: String,

    // Socks port
    #[arg(long, default_value_t = 1080)]
    pub socks_port: u16,

    // Enable tracing with tokio-console
    #[arg(long, default_value_t = false)]
    pub tracing: bool,
}
