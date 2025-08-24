use std::{collections::HashMap, os::fd::AsRawFd, sync::Arc};

use clap::Parser;
use log::{error, info};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};
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

async fn init_local_tcp_proxy(method: Arc<Box<dyn BaseMethod + Send + Sync>>, args: Args) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", LOCAL_TCP_PORT))
        .await
        .expect("Failed to bind address");
    loop {
        let (mut ingress, _) = listener.accept().await.expect("Failed to accept request");
        let orginal_dst = method
            .get_original_dst(ingress.as_raw_fd())
            .expect("Failed to get orignal destination");
        let (ssh_in, ssh_out) = ssh(
            &args.user,
            &args.host,
            args.port,
            &args.identity_file,
            &format!("remote_proxy -p tcp -d {}", orginal_dst),
        )
        .expect("Failed to create ssh connection to remote host");
        let mut egress = utils::IOWrapper {
            tx: utils::Tx::Child(ssh_in),
            rx: utils::Rx::Child(ssh_out),
        };
        tokio::spawn(async move {
            match tokio::io::copy_bidirectional(&mut ingress, &mut egress).await {
                Ok((to_egress, to_ingress)) => {
                    info!(
                        "Connection ended gracefully ({to_egress} bytes from client, {to_ingress} bytes from server)"
                    )
                }
                Err(err) => {
                    error!("Error while proxying: {err}");
                }
            }
        });
    }
}

async fn init_local_udp_proxy(method: Arc<Box<dyn BaseMethod + Send + Sync>>, args: Args) {
    let ingress = UdpSocket::bind(format!("0.0.0.0:{}", LOCAL_UDP_PORT))
        .await
        .expect("Failed to bind address");

    let mut channels = HashMap::new();
    loop {
        let mut buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
        let (_nbytes, addr) = ingress
            .recv_from(&mut buf)
            .await
            .expect("Failed to receive packet");
        if !channels.contains_key(&addr) {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
            let egress = UdpSocket::bind(addr)
                .await
                .expect("Failed to bind to origin socket");
            let orginal_dst = method
                .get_original_dst(egress.as_raw_fd())
                .expect("Failed to get original destination");
            let (mut ssh_in, mut ssh_out) = ssh(
                &args.user,
                &args.host,
                args.port,
                &args.identity_file,
                &format!("remote_proxy -p udp -d {}", orginal_dst),
            )
            .expect("Failed to create ssh connection to remote host");
            tokio::spawn(async move {
                let mut rx_buf = Vec::with_capacity(DATAGRAM_MAXSIZE);
                loop {
                    tokio::select! {
                        Some(tx_buf) = rx.recv() => {
                            ssh_in.write(&tx_buf).await.expect("Failed to write buffer");
                        }
                        Ok(nbytes) = ssh_out.read(&mut rx_buf) => {
                            egress
                                .send(&rx_buf[..nbytes])
                                .await
                                .expect("Failed to send buffer to local client");
                        }
                    }
                }
            });
            channels.insert(addr, tx);
        }
        channels
            .get_mut(&addr)
            .expect(&format!("Key {} doesn't exist", &addr))
            .send(buf)
            .await
            .expect("Failed to transfer buffer");
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

    if args.dns {
        let _args = args.clone();
        tokio::spawn(init_local_udp_proxy(arc_m.clone(), _args));
    }
    init_local_tcp_proxy(arc_m.clone(), args).await;
}
