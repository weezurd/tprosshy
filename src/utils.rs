use chrono::Local;
use env_logger;
use env_logger::Env;
use log::warn;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddrV4;
use std::process::Command;

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

pub(crate) fn get_original_dst(sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>> {
    let dst_addr_v4 = sock_ref
        .original_dst_v4()
        .expect("Failed to get orginal destination")
        .as_socket_ipv4()
        .expect("Failed to convert original destination to ipv4");
    return Ok(dst_addr_v4);
}

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
