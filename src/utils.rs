use chrono::Local;
use core::task::Context;
use core::task::Poll;
use env_logger;
use env_logger::Env;
use log::warn;
use resolv_conf::Config;
use resolv_conf::ScopedIp;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddrV4;
use std::pin::Pin;
use tokio::io::{self, AsyncRead, AsyncWrite, ReadBuf, Stdin, Stdout};
use tokio::process::{ChildStdin, ChildStdout};

pub fn init_logger(file_path: Option<String>) {
    let env = Env::default().filter_or("RUST_LOG", "debug");
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

pub enum Tx {
    Child(ChildStdin),
    Std(Stdout),
}

pub enum Rx {
    Child(ChildStdout),
    Std(Stdin),
}

pub struct IOWrapper {
    pub tx: Tx,
    pub rx: Rx,
}

impl IOWrapper {
    pub fn default() -> Self {
        return IOWrapper {
            tx: Tx::Std(tokio::io::stdout()),
            rx: Rx::Std(tokio::io::stdin()),
        };
    }
}

impl AsyncRead for IOWrapper {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.get_mut().rx {
            Rx::Child(x) => Pin::new(x).poll_read(cx, buf),
            Rx::Std(x) => Pin::new(x).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for IOWrapper {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().tx {
            Tx::Child(x) => Pin::new(x).poll_write(cx, data),
            Tx::Std(x) => Pin::new(x).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().tx {
            Tx::Child(x) => Pin::new(x).poll_flush(cx),
            Tx::Std(x) => Pin::new(x).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().tx {
            Tx::Child(x) => Pin::new(x).poll_shutdown(cx),
            Tx::Std(x) => Pin::new(x).poll_shutdown(cx),
        }
    }
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
