use log::{info, warn};
use std::{net::SocketAddrV4, sync::Arc};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
};

use crate::{LOCAL_TCP_PORT, methods::BaseMethod};
use fast_socks5::client::Socks5Stream;
use tokio_util::sync::CancellationToken;

pub async fn init_proxy(
    method: Arc<Box<dyn BaseMethod + Send + Sync>>,
    token: CancellationToken,
    socks_addr: SocketAddrV4,
) {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", LOCAL_TCP_PORT))
        .await
        .expect("Failed to bind address");
    let task_tracker = tokio_util::task::TaskTracker::new();

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                let orginal_dst = method
                    .get_original_dst(socket2::SockRef::from(&ingress))
                    .expect("Failed to get orignal destination");
                task_tracker.spawn(handle_tcp(ingress, orginal_dst, token.clone(), socks_addr));
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                break
            }
        }
    }
}

async fn handle_tcp(
    mut stream: TcpStream,
    destination: SocketAddrV4,
    token: CancellationToken,
    socks_addr: SocketAddrV4,
) {
    let mut remote_config = fast_socks5::client::Config::default();
    remote_config.set_skip_auth(false);
    if let Ok(mut remote) = Socks5Stream::connect(
        socks_addr,
        destination.ip().to_string(),
        destination.port(),
        remote_config,
    )
    .await
    {
        info!("New connection opened. Destination: {}", destination);
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
