use clap::{Parser, ValueEnum};
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

async fn init_remote_tcp_proxy(destination: &str) {
    let mut stream = match TcpStream::connect(destination).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to destination: {}", e);
            return;
        }
    };
    let mut stdio = utils::IOWrapper::default();
    let _ = tokio::io::copy_bidirectional(&mut stdio, &mut stream).await;
}

async fn init_remote_udp_proxy(destination: &str) {
    let (mut stdin, mut stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    sock.connect(destination)
        .await
        .expect("Failed to connect to destination address");
    let mut tx_buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
    let mut rx_buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
    loop {
        tokio::select! {
            Ok(nbytes) = stdin.read(&mut tx_buf) => {
                sock.send(&tx_buf[..nbytes]).await.expect("Failed to send packet");
            }
            Ok(nbytes) = sock.recv(&mut rx_buf) => {
                stdout.write(&tx_buf[..nbytes]).await.expect("Failed to write buffer");
            }

        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    utils::init_logger();

    match &args.protocol {
        Protocol::TCP => init_remote_tcp_proxy(&args.destination).await,
        Protocol::UDP => init_remote_udp_proxy(&args.destination).await,
    };
}
