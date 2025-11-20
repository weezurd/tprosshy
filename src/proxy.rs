use log::{error, info, warn};
use std::net::SocketAddrV4;
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
};

use crate::get_original_dst;
use fast_socks5::client::Socks5Stream;
use tokio_util::sync::CancellationToken;

pub async fn init_proxy(
    token: CancellationToken,
    socks_addr: SocketAddrV4,
    tcp_listener: TcpListener,
) {
    let task_tracker = tokio_util::task::TaskTracker::new();
    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_addr));
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                break
            }
        }
    }
}

async fn handle_tcp(mut stream: TcpStream, token: CancellationToken, socks_addr: SocketAddrV4) {
    let dst = match get_original_dst(socket2::SockRef::from(&stream)) {
        Ok(addr) => addr,
        Err(_) => {
            error!("Failed to get original destination of NAT-ed address");
            return;
        }
    };
    if let Ok(mut remote) = Socks5Stream::connect(
        socks_addr,
        dst.ip().to_string(),
        dst.port(),
        fast_socks5::client::Config::default(),
    )
    .await
    {
        info!("New connection opened. Destination: {}", dst);
        tokio::select! {
            _ = copy_bidirectional(&mut stream, &mut remote) => {}
            _ = token.cancelled() => {
                return
            }
        }
    } else {
        warn!("Failed to init connection to socks5 server");
    }
    info!("Connection closed.");
}
