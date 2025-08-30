use clap::{Parser, ValueEnum};
use log::info;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tprosshy::{DATAGRAM_MAXSIZE, utils};

#[derive(Debug, Clone, ValueEnum)]
enum Protocol {
    TCP,
    UDP,
}

/// Transparent proxy over ssh. Remote proxy.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Protocol
    #[arg(value_enum, short, long)]
    protocol: Protocol,

    /// Destination address
    #[arg(short, long)]
    destination: String,
}

async fn init_remote_tcp_proxy(destination: String) {
    info!("Received connection");
    let mut egress = match TcpStream::connect(destination).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to destination: {}", e);
            return;
        }
    };
    let mut ingress = utils::IOWrapper::default();
    let _ = tokio::io::copy_bidirectional(&mut ingress, &mut egress).await;
}

async fn init_remote_udp_proxy(destination: String) {
    let (mut stdin, mut stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    sock.connect(destination)
        .await
        .expect("Failed to connect to destination address");
    let mut buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
    let mut nbytes: usize;

    nbytes = stdin.read(&mut buf).await.expect("Failed to read buffer");
    sock.send(&buf[..nbytes])
        .await
        .expect("Failed to send packet");

    buf.clear();

    nbytes = sock.recv(&mut buf).await.expect("Failed to receive packet");
    stdout
        .write(&buf[..nbytes])
        .await
        .expect("Failed to write buffer");
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    utils::init_logger(Some("/tmp/tprosshy.log".to_string()));
    let task_tracker = tokio_util::task::TaskTracker::new();

    match &args.protocol {
        Protocol::TCP => task_tracker.spawn(init_remote_tcp_proxy(args.destination)),
        Protocol::UDP => task_tracker.spawn(init_remote_udp_proxy(args.destination)),
    };

    task_tracker.close();
    task_tracker.wait().await;
    info!("Connection closed");
}
