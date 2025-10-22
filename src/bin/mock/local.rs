use log::{debug, error, info, warn};
use std::io::{Cursor, ErrorKind};
use std::net::Ipv4Addr;
use std::time::Duration;
use std::{net::SocketAddrV4, sync::Arc};
use tokio::io::{self};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tprosshy::{
    LOCAL_TCP_PORT, LOCAL_UDP_PORT, get_available_method, init_local_proxy, scp, ssh,
    utils::{self},
};

const BUFSIZE: usize = usize::pow(2, 14);
const IP_RANGE: &'static str = "10.10.10.10/32";
const DESTINATION: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 10, 10, 10), 8068);
const HOST: &'static str = "homelab";

#[tokio::main]
async fn main() {
    utils::init_logger(None);
    scp(
        &HOST,
        "target/x86_64-unknown-linux-musl/release/remote_mock",
        "/tmp/remote",
    )
    .await
    .expect("Failed to upload executable to remote");
    let arc_m = Arc::new(get_available_method());
    arc_m
        .setup_fw(&IP_RANGE, LOCAL_TCP_PORT, LOCAL_UDP_PORT)
        .expect("Failed to setup firewall");

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    task_tracker.spawn(init_local_proxy(
        arc_m.clone(),
        ssh(&HOST, "/tmp/remote"),
        token.clone(),
    ));
    info!("Local proxy started");

    // Simulate traffic
    task_tracker.spawn(request_tcp(token.clone()));
    // task_tracker.spawn(request_dns(token.clone()));
    task_tracker.close();

    // Wait for a while
    // tokio::time::sleep(Duration::from_secs(10)).await;
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received. Attempt to gracefully shutdown.");

    token.cancel();
    task_tracker.wait().await;
    arc_m.restore_fw().expect("Failed to restore firewall");
    info!("Local proxy finished");
}

async fn request_tcp(token: CancellationToken) {
    info!("Testing TCP request...");
    if let Ok(egress) = TcpStream::connect(&DESTINATION).await {
        info!("Destination connected. Starting TCP loop..");
        let (mut reader, mut writer) = io::split(egress);
        let task_tracker = tokio_util::task::TaskTracker::new();

        // Read thread
        let t1 = token.clone();
        task_tracker.spawn(async move {
            loop {
                let mut buf = [0u8; BUFSIZE];
                tokio::select! {
                    read_result = reader.read(&mut buf) => {
                        match read_result {
                            Err(e) if e.kind() == ErrorKind::ConnectionAborted => {
                                debug!("Client disconnected. Maybe RST");
                            }
                            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                                debug!("Client disconnected. Maybe FIN");
                            }
                            Err(e) => {
                                warn!("Client disconnected. Unexpected error: {}", e);
                            }
                            Ok(nread) if nread == 0 => {
                                debug!("Channel closed with EOF");
                            }
                            Ok(nread) => {
                                info!("Received: {:?}", &buf[..nread]);
                                continue
                            }
                        }
                        break
                    }
                    _ = t1.cancelled() => {
                        break
                    }
                }
            }
        });

        // Write thread
        let t2 = token.clone();
        task_tracker.spawn(async move {
            loop {
                let mut buffer = Cursor::new([123u8; 500 as usize]);
                tokio::select! {
                    write_result = writer.write_all_buf(&mut buffer) => {
                        match write_result {
                            Err(e) => {debug!("Failed to write: {}", e);}
                            Ok(()) => {}
                        }
                    }
                    _ = t2.cancelled() => {break}
                }
                sleep(Duration::from_secs(1)).await;
            }
        });

        task_tracker.close();
        task_tracker.wait().await;
    } else {
        warn!("Failed to connect to destination");
    }
}

async fn request_dns(token: CancellationToken) {
    info!("Testing DNS request...");

    if let Ok(egress) = UdpSocket::bind(format!("0.0.0.0:0")).await
        && let Ok(_) = egress.connect("127.0.0.53:53").await
    {
        let msg = b"hello_udp";
        if egress.send(msg).await.is_err() {
            error!("Failed to send buffer");
            return;
        }

        loop {
            let mut buf = [0u8; BUFSIZE];
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::select! {
                recv_result = egress.recv(&mut buf) => {
                    match recv_result {
                        Ok(nread) => {
                            info!("Received: {}", str::from_utf8(&buf[..nread]).unwrap());
                            if egress.send(msg).await.is_err() {
                                error!("Failed to write buffer");
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to receive buffer: {}", e);
                        }
                    }
                    break
                }
                _ = token.cancelled() => {
                    break
                }
            }
        }
    } else {
        warn!("Failed to connect to destination");
    }
}
