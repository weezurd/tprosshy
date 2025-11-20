use chrono::Local;
use env_logger;
use env_logger::Env;
use log::warn;
use resolv_conf::Config;
use resolv_conf::ScopedIp;
use std::error::Error;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddrV4;

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

pub fn get_system_resolvers() -> Vec<SocketAddrV4> {
    let mut dns_resolvers: Vec<SocketAddrV4> = vec![];
    if let Ok(content) = fs::read("/etc/resolv.conf")
        && let Ok(config) = Config::parse(&content)
    {
        for ns in config.nameservers {
            match ns {
                ScopedIp::V4(addr) => {
                    dns_resolvers.push(SocketAddrV4::new(addr, 53));
                }
                _ => {
                    warn!("Found IPv6 nameserver. Only Ipv4 is supported for now")
                }
            }
        }
    }

    return dns_resolvers;
}

pub fn get_original_dst(sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>> {
    let dst_addr_v4 = sock_ref
        .original_dst_v4()
        .expect("Failed to get orginal destination")
        .as_socket_ipv4()
        .expect("Failed to convert original destination to ipv4");
    return Ok(dst_addr_v4);
}
