use std::io::ErrorKind;

use log::{debug, info, warn};
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
const BUFSIZE: usize = usize::pow(2, 14);

#[tokio::main]
async fn main() {
    utils::init_logger(Some("/tmp/tprosshy.log".to_string()));
    let task_tracker = tokio_util::task::TaskTracker::new();
    let token = CancellationToken::new();

    info!("Remote proxy started");

    let tcp_listener = TcpListener::bind(format!("10.10.10.10:{}", MOCK_TCP_PORT))
        .await
        .expect("Failed to bind address");

    loop {
        tokio::select! {
            _ = init_remote_proxy(token.clone()) => {break}
            Ok((ingress, addr)) = tcp_listener.accept() => {
                info!("New client connected: {}", addr);
                task_tracker.spawn(handle_tcp(ingress, token.clone()));
            }
        }
    }

    token.cancel();
    task_tracker.close();
    task_tracker.wait().await;
    info!("Remote proxy finished");
}

async fn handle_tcp(mut ingress: TcpStream, token: CancellationToken) {
    loop {
        let mut buf = [0; BUFSIZE];
        tokio::select! {
            read_result = ingress.read(&mut buf) => {
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
                        // debug!("nread = 0. Channel closed");
                    }
                    Ok(nread) => {
                        info!("Received: {}", str::from_utf8(&buf[..nread]).unwrap());
                        // let n = ingress.write(&buf[..nread]).await.unwrap();
                        match ingress.write(b"echo_hello_tcp\n").await {
                            Ok(n) => {info!("Sent {} bytes", n)}
                            Err(e) => {info!("Failed to sent: {}", e)}
                        }
                        let _ = ingress.flush().await;
                        info!("Sent: echo_hello_tcp");
                        continue
                    }
                }
                // break
            }
            _ = token.cancelled() => {break}
        }
    }
}
