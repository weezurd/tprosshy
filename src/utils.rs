use chrono::Local;
use core::task::Context;
use core::task::Poll;
use env_logger;
use log::LevelFilter;
use std::fs::OpenOptions;
use std::io::Write;
use std::pin::Pin;
use tokio::io::{self, AsyncRead, AsyncWrite, ReadBuf, Stdin, Stdout};
use tokio::process::{ChildStdin, ChildStdout};

pub fn init_logger(file_path: Option<String>) {
    let mut logger = env_logger::Builder::new();
    logger
        .format(|buf, record| {
            writeln!(
                buf,
                "{} {:<30} {:>7} {}",
                Local::now().format("%Y-%m-%dT%H:%M:%S%.3f"),
                format!(
                    "{}:{}",
                    record.file().unwrap_or("unknown"),
                    record.line().unwrap_or(0)
                ),
                format!("[{}]", record.level()),
                record.args()
            )
        })
        .filter(None, LevelFilter::Info);

    if let Some(path) = file_path {
        let target = Box::new(
            OpenOptions::new()
                .create(true)
                .append(true)
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
