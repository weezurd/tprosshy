use std::{os::fd::AsRawFd, sync::Arc};

use clap::Parser;
use log::{error, info};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};

use tokio_util::sync::CancellationToken;
use tprosshy::{
    BaseMethod, DATAGRAM_MAXSIZE, LOCAL_TCP_PORT, LOCAL_UDP_PORT, get_available_method, scp, ssh,
    utils,
};

/// Transparent proxy over ssh. Local proxy.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// Remote user
    #[arg(short, long)]
    user: String,

    /// Remote host
    #[arg(short, long)]
    host: String,

    /// SSH port
    #[arg(short, long, default_value_t = 22)]
    port: u16,

    /// Identity file
    #[arg(short, long, default_value_t = String::from("~/.ssh/id_ed25519"))]
    identity_file: String,

    /// Allowed IP range
    #[arg(short, long, default_value_t = String::from("0.0.0.0/0"))]
    ip_range: String,

    /// Enable dns proxy
    #[arg(short, long, default_value_t = true)]
    dns: bool,
}

async fn init_local_tcp_proxy(
    method: Arc<Box<dyn BaseMethod + Send + Sync>>,
    args: Args,
    token: CancellationToken,
) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", LOCAL_TCP_PORT))
        .await
        .expect("Failed to bind address");
    let task_tracker = tokio_util::task::TaskTracker::new();

    loop {
        tokio::select! {
            Ok((mut ingress, _)) = listener.accept() => {
                let orginal_dst = method
                    .get_original_dst(ingress.as_raw_fd())
                    .expect("Failed to get orignal destination");
                let mut proc = ssh(
                    &args.user,
                    &args.host,
                    args.port,
                    &args.identity_file,
                    &format!("remote_proxy -p tcp -d {}", orginal_dst),
                );
                let ssh_in = proc.stdin.take().expect("Failed to acquire stdin");
                let ssh_out = proc.stdout.take().expect("Failed to acquire stdout");
                let mut egress = utils::IOWrapper {
                    tx: utils::Tx::Child(ssh_in),
                    rx: utils::Rx::Child(ssh_out),
                };
                let local_token = token.clone();
                task_tracker.spawn(async move {
                    tokio::select! {
                        x = tokio::io::copy_bidirectional(&mut ingress, &mut egress) => {
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
                        _ = local_token.cancelled() => {
                            let _ = proc.start_kill();
                        }
                    }
                });
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                break
            }
        }
    }
}

async fn init_local_udp_proxy(
    method: Arc<Box<dyn BaseMethod + Send + Sync>>,
    args: Args,
    token: CancellationToken,
) {
    let ingress = UdpSocket::bind(format!("0.0.0.0:{}", LOCAL_UDP_PORT))
        .await
        .expect("Failed to bind address");

    let task_tracker = tokio_util::task::TaskTracker::new();

    loop {
        let mut buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
        tokio::select! {
            Ok((_nbytes, addr)) = ingress.recv_from(&mut buf) => {
                let egress = UdpSocket::bind(addr)
                    .await
                    .expect("Failed to bind to origin socket");
                let orginal_dst = method
                    .get_original_dst(egress.as_raw_fd())
                    .expect("Failed to get original destination");
                let mut proc = ssh(
                    &args.user,
                    &args.host,
                    args.port,
                    &args.identity_file,
                    &format!("remote_proxy -p udp -d {}", orginal_dst),
                );
                task_tracker.spawn(async move {
                    let mut ssh_in = proc.stdin.take().expect("Failed to acquire stdin");
                    let mut ssh_out = proc.stdout.take().expect("Failed to acquire stdout");
                    ssh_in.write(&buf).await.expect("Failed to write buffer");

                    buf.clear();

                    let nbytes = ssh_out.read(&mut buf).await.expect("Failed to read buffer");
                    egress
                        .send(&buf[..nbytes])
                        .await
                        .expect("Failed to send buffer to local client");
                });
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                break
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    utils::init_logger();
    scp(
        &args.user,
        &args.host,
        args.port,
        &args.identity_file,
        "remote_proxy",
        "remote_proxy",
    )
    .await
    .expect("Failed to upload executable to remote");
    let arc_m = Arc::new(get_available_method());
    arc_m
        .setup_fw(&args.ip_range, LOCAL_TCP_PORT)
        .expect("Failed to setup firewall");

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    if args.dns {
        task_tracker.spawn(init_local_udp_proxy(
            arc_m.clone(),
            args.clone(),
            token.clone(),
        ));
    }

    task_tracker.spawn(init_local_tcp_proxy(
        arc_m.clone(),
        args.clone(),
        token.clone(),
    ));
    task_tracker.close();

    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received. Attempt to gracefully shutdown...");
    token.cancel();
    task_tracker.wait().await;
    info!("Bye.");
}
