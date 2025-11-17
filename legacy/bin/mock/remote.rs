use log::{debug, info, warn};
use std::io::ErrorKind;
use tokio::io::{self};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use tprosshy::{
    init_remote_proxy,
    utils::{self},
};
const MOCK_TCP_PORT: u16 = 8068;

#[tokio::main]
async fn main() {
    utils::init_logger(Some("/tmp/tprosshy.log".to_string()));
    let task_tracker = tokio_util::task::TaskTracker::new();
    let token = CancellationToken::new();

    info!("Remote proxy started");

    let tcp_listener = TcpListener::bind(format!("10.10.10.10:{}", MOCK_TCP_PORT))
        .await
        .expect("Failed to bind address");

    let t1 = token.clone();
    task_tracker.spawn(async move {
        let local_task_tracker = tokio_util::task::TaskTracker::new();
        loop {
            tokio::select! {
                Ok((ingress, addr)) = tcp_listener.accept() => {
                    info!("New client connected: {}", addr);
                    local_task_tracker.spawn(handle_tcp(ingress, t1.clone()));
                }
                _ = t1.cancelled() => {break}
            }
        }
        local_task_tracker.close();
        local_task_tracker.wait().await;
    });

    let _ = init_remote_proxy(token.clone()).await;
    token.cancel();
    task_tracker.close();
    task_tracker.wait().await;
    info!("Remote proxy finished");
}

async fn handle_tcp(ingress: TcpStream, token: CancellationToken) {
    let (mut reader, mut writer) = io::split(ingress);

    loop {
        let mut buf = [0; 500];
        tokio::select! {
            read_result = reader.read(&mut buf) => {
                match read_result {
                    Err(e)
                    if e.kind() == ErrorKind::ConnectionAborted
                    || e.kind() == ErrorKind::ConnectionReset =>
                    {
                        debug!("Client disconnected. Maybe RST");
                    }
                    Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                        debug!("Client disconnected. Maybe FIN");
                    }
                    Err(e) => {
                        warn!("Client disconnected. Unexpected error: {}", e);
                    }
                    Ok(nread) if nread == 0 => {
                        debug!("nread = 0. Channel closed");
                    }
                    Ok(nread) => {
                        match writer.write_all(&buf[..nread]).await {
                            Ok(_) => {info!("Sent {} bytes", nread)}
                            Err(e) => {info!("Failed to sent: {}", e)}
                        }
                        continue
                    }
                }
                break
            }
            _ = token.cancelled() => {break}
        }
    }
}
