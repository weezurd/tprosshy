use std::collections::HashMap;

use log::{info, warn};
use std::net::SocketAddrV4;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tprosshy::{
    BUFSIZE, Frame, FrameType, Header, Protocol,
    utils::{self},
};

use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

async fn mux(token: CancellationToken) {
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let mut ingress = utils::IOWrapper {
        tx: utils::Tx::Std(stdout),
        rx: utils::Rx::Std(stdin),
    };
    let (mux_tx, mut mux_rx) = tokio::sync::mpsc::channel::<Frame>(1);

    let task_tracker = tokio_util::task::TaskTracker::new();
    let mut tx_pool = HashMap::new();

    let mut raw_buf = [0u8; size_of::<Frame>()];
    loop {
        tokio::select! {
            read_result = ingress.read_exact(&mut raw_buf) => {
                if read_result.is_err() {
                    warn!("Ingress closed with error: {}", read_result.err().unwrap());
                    return
                }

                info!("Received some bytes");
                let (frame, _) = Frame::deserialize(&raw_buf).expect("Failed to deserialize raw buffer");
                match frame.header.ftype {
                    FrameType::Rst => return,
                    _ => {
                        if !tx_pool.contains_key(&frame.header.id) {
                            let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
                            tx_pool.insert(frame.header.id, tx);

                            match frame.header.protocol {
                                Protocol::TCP => {
                                    task_tracker.spawn(handle_tcp(
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
                                        token.clone(),
                                        frame.header.dst,
                                    ));
                                }
                                Protocol::UDP => {
                                    task_tracker.spawn(handle_udp(
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
                                        frame.header.dst,
                                    ));
                                }
                            }
                        }
                        tx_pool
                            .get_mut(&frame.header.id)
                            .expect("Failed to get receiver")
                            .send(frame.payload[..frame.header.size as usize].to_vec())
                            .await
                            .expect("Failed to send buffer");
                    }
                }
            }
            Some(frame) = mux_rx.recv() => {
                if let Ok(_) = frame.serialize(&mut raw_buf) {
                    ingress.write_all(&raw_buf).await.expect("Failed to write buffer");
                    ingress.flush().await.expect("Failed to flush buffer");
                };
            }
            _ = token.cancelled() => {
                return
            }
        }
    }
}

async fn handle_tcp(
    tx: Sender<Frame>,
    mut rx: Receiver<Vec<u8>>,
    id: u32,
    token: CancellationToken,
    destination: SocketAddrV4,
) {
    info!("Received request to connect to {}", destination);
    let mut egress = TcpStream::connect(destination)
        .await
        .expect("Failed to connect to destination");
    loop {
        let mut buf = [0u8; BUFSIZE];
        tokio::select! {
            read_result = egress.read(&mut buf) => {
                info!("Received some bytes from egress..");
                if let Ok(nread) = read_result && nread > 0 {
                    let frame = Frame {
                        header: Header {
                            id: id,
                            ftype: FrameType::Data,
                            protocol: Protocol::TCP,
                            size: nread as u32,
                            dst: destination,
                        },
                        payload: buf
                    };
                    tx.send(frame).await.expect("Failed to send frame");
                } else {
                    if read_result.is_err() {
                        warn!("Channel closed with error: {}", read_result.unwrap_err());
                    } else {
                        info!("Channel closed");
                    }
                    return
                }
            }
            Some(b) = rx.recv() => {
                info!("Writing {} bytes to egress", b.len());
                egress.write_all(&b).await.expect("Failed to write buffer");
                egress.flush().await.expect("Failed to flush buffer");
            }
            _ = token.cancelled() => {
                return
            }
        }
    }
}

async fn handle_udp(
    tx: Sender<Frame>,
    mut rx: Receiver<Vec<u8>>,
    id: u32,
    destination: SocketAddrV4,
) {
    let egress = UdpSocket::bind(format!("0.0.0.0:0"))
        .await
        .expect("Failed to bind address");

    egress
        .connect(destination)
        .await
        .expect("Failed to connect to destination");

    let b = rx.recv().await.expect("Failed to receive buffer");
    egress.send(&b).await.expect("Failed to write buffer");

    let mut buf = [0u8; BUFSIZE];
    let (nrecv, _) = egress
        .recv_from(&mut buf)
        .await
        .expect("Failed to receive buffer");
    let frame = Frame {
        header: Header {
            id: id,
            ftype: FrameType::Data,
            protocol: Protocol::UDP,
            size: nrecv as u32,
            dst: destination,
        },
        payload: buf,
    };
    tx.send(frame).await.expect("Failed to send frame");
}

async fn init_remote_proxy(token: CancellationToken) {
    mux(token).await;
}

#[tokio::main]
async fn main() {
    utils::init_logger(Some("/tmp/tprosshy.log".to_string()));
    let task_tracker = tokio_util::task::TaskTracker::new();
    let token = CancellationToken::new();

    task_tracker.spawn(init_remote_proxy(token));
    task_tracker.close();
    info!("Remote proxy started.");
    task_tracker.wait().await;
    info!("Remote proxy finished.");
}
