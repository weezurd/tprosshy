mod methods;
pub use methods::get_available_method;

mod ssh;
pub use ssh::{scp, ssh};

mod magic;
pub use magic::*;

pub mod utils;

mod frame;

mod local_proxy;
pub use local_proxy::init_local_proxy;

mod remote_proxy;
pub use remote_proxy::init_remote_proxy;

use clap::Parser;

/// Transparent proxy over ssh. Local proxy.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Remote user
    #[arg(short, long)]
    pub user: String,

    /// Remote host. Only IPv4 is supported for now.
    #[arg(short, long)]
    pub host: String,

    /// SSH port
    #[arg(short, long, default_value_t = 22)]
    pub port: u16,

    /// Identity file
    #[arg(short, long, default_value_t = String::from("~/.ssh/id_ed25519"))]
    pub identity_file: String,

    /// Allowed IP range
    #[arg(short, long, default_value_t = String::from("0.0.0.0/0"))]
    pub ip_range: String,

    /// Enable dns proxy
    #[arg(short, long, default_value_t = true)]
    pub dns: bool,
}
