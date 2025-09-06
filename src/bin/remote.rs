use clap::{Parser, ValueEnum};
use log::{error, info};
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
    let mut egress = match TcpStream::connect(destination).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to destination: {}", e);
            return;
        }
    };
    let mut ingress = utils::IOWrapper::default();
    let x = tokio::io::copy_bidirectional(&mut ingress, &mut egress).await;
    match x {
        Ok((to_egress, to_ingress)) => {
            info!(
                "Connection ended gracefully ({to_egress} bytes from client, {to_ingress} bytes from server)"
            )
        }
        Err(err) => {
            error!("Error while proxying: {err}");
        }
    }
}

async fn init_remote_udp_proxy(destination: String) {
    let (mut stdin, mut stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    sock.connect(destination)
        .await
        .expect("Failed to connect to destination address");
    info!("Connected to destination");
    let mut buf_size_raw = (0 as usize).to_be_bytes();
    stdin
        .read_exact(&mut buf_size_raw)
        .await
        .expect("Failed to read buffer");
    let buf_size: usize = usize::from_be_bytes(buf_size_raw);
    info!("Going to read {} bytes from tunnel", buf_size);

    let mut buf = vec![0 as u8; buf_size];
    let mut nbytes = stdin
        .read_exact(&mut buf)
        .await
        .expect("Failed to read buffer");
    info!("Received {} bytes from ssh tunnel", nbytes);

    sock.send(&buf[..nbytes])
        .await
        .expect("Failed to send packet");

    let mut buf = [0 as u8; DATAGRAM_MAXSIZE];
    nbytes = sock.recv(&mut buf).await.expect("Failed to receive packet");
    stdout
        .write(&nbytes.to_be_bytes())
        .await
        .expect("Failed to write buffer");
    stdout.flush().await.expect("Failed to flush stdout");
    info!("Going to send {} bytes to local", nbytes);

    stdout
        .write(&buf[..nbytes])
        .await
        .expect("Failed to write buffer");
    stdout.flush().await.expect("Failed to flush stdout");
    info!("Sent {} bytes to local", nbytes);
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
    info!("Remote proxy started.");
    task_tracker.wait().await;
    info!("Remote proxy finished.");
}
