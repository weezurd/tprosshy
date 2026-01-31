use chrono::Local;
use env_logger;
use env_logger::Env;
use log::warn;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddrV4;
use std::process::Command;

/// Initializes the global logger.
///
/// This function configures `env_logger` with:
/// - A timestamped log format including module path and line number
/// - Log level controlled by the `RUST_LOG` environment variable
/// - Optional file output instead of stdout
///
/// If a log file path is provided, the file is created (or truncated if it
/// already exists) and used as the logging target.
///
/// # Parameters
/// - `file_path`: Optional path to a log file. If `None`, logs are written
///   to standard output.
///
/// # Behavior
/// - Defaults log level to `info` if `RUST_LOG` is not set.
/// - Panics if the log file cannot be created.
///
/// # Notes
/// - Timestamps use local time.
/// - Log formatting is optimized for human readability, not structured logging.
pub fn init_logger(file_path: Option<String>) {
    let env = Env::default().filter_or("RUST_LOG", "info");
    let mut logger = env_logger::Builder::from_env(env);
    logger.format(|buf, record| {
        writeln!(
            buf,
            "{} {:<30} {:>7} {}",
            Local::now().format("%Y-%m-%dT%H:%M:%S%.3f"),
            format!(
                "{}:{}",
                record.module_path().unwrap_or("unknown"),
                record.line().unwrap_or(0)
            ),
            format!("[{}]", record.level()),
            record.args()
        )
    });

    if let Some(path) = file_path {
        let target = Box::new(
            OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(true)
                .open(path)
                .expect("Failed to create log file"),
        );
        logger.target(env_logger::Target::Pipe(target));
    }

    logger.init();
}

/// Retrieves the original destination address of a NAT-ed TCP socket.
///
/// This function uses `SO_ORIGINAL_DST` to determine the original IPv4
/// destination of a connection that has been transparently redirected
/// (e.g., via iptables or nftables).
///
/// # Parameters
/// - `sock_ref`: A reference to the TCP socket.
///
/// # Returns
/// - `Ok(SocketAddrV4)` containing the original destination address.
/// - `Err(Box<dyn Error>)` if the destination cannot be determined.
///
/// # Panics
/// - Panics if the socket does not expose an IPv4 original destination.
///
/// # Limitations
/// - IPv4 only.
/// - Linux-specific; relies on kernel NAT behavior.
pub(crate) fn get_original_dst(sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>> {
    let dst_addr_v4 = sock_ref
        .original_dst_v4()
        .expect("Failed to get orginal destination")
        .as_socket_ipv4()
        .expect("Failed to convert original destination to ipv4");
    return Ok(dst_addr_v4);
}

/// Retrieves the first local DNS nameserver from `/etc/resolv.conf`.
///
/// This function executes an `awk` command to extract the first `nameserver`
/// entry from `/etc/resolv.conf`. It is intended as a simple, best-effort
/// helper and does not attempt to handle more complex resolver setups.
///
/// # Returns
/// - `Some(String)` containing the nameserver address.
/// - `None` if:
///   - The command execution fails
///   - No nameserver is found
///   - The command exits with an error
///
/// # Caveats
/// - Assumes a Unix-like system with `awk` available.
/// - Only the first `nameserver` entry is returned.
pub(crate) fn get_local_nameserver() -> Option<String> {
    let output = match Command::new("awk")
        .arg("$1 == \"nameserver\" {print $2; exit}")
        .arg("/etc/resolv.conf")
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            warn!("Failed to execute awk command: {}", e);
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("Awk exited with error: {}", stderr);
        return None;
    }

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if result.is_empty() {
        warn!("Awk found no nameserver in /etc/resolv.conf");
        return None;
    }

    Some(result)
}
