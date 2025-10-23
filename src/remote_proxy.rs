use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    BUFSIZE,
    frame::{Frame, FrameType, Header, Protocol},
    utils::{self},
};
use log::{debug, error, info, warn};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

async fn mux(token: CancellationToken) {
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let mut ingress = utils::IOWrapper {
        tx: utils::Tx::Std(stdout),
        rx: utils::Rx::Std(stdin),
    };
    let (mux_tx, mut mux_rx) = tokio::sync::mpsc::channel::<Frame>(1);
    let mut join_set = tokio::task::JoinSet::new();
    let mut tx_pool = HashMap::new();
    let mut raw_buf = [0u8; size_of::<Frame>()];
    let dns_resolvers = Arc::new(utils::get_system_resolvers());

    info!("Start demuxing task");
    loop {
        tokio::select! {
            read_result = ingress.read_exact(&mut raw_buf) => {
                if read_result.is_err() {
                    warn!("Ingress closed with error: {}", read_result.err().unwrap());
                    return
                }

                let (frame, _) = Frame::deserialize(&raw_buf).expect("Failed to deserialize raw buffer");
                match frame.header.ftype {
                    FrameType::Rst => return,
                    _ => {
                        if !tx_pool.contains_key(&frame.header.id) {
                            let (tx, rx) = tokio::sync::mpsc::channel::<Frame>(1);
                            tx_pool.insert(frame.header.id, tx);

                            match frame.header.protocol {
                                Protocol::TCP => {
                                    join_set.spawn(handle_tcp(
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
                                        token.clone(),
                                        frame.header.dst,
                                    ));
                                }
                                Protocol::DNS => {
                                    join_set.spawn(handle_dns(
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
                                        dns_resolvers.clone()
                                    ));
                                }
                            }
                        }
                        if tx_pool
                            .get_mut(&frame.header.id)
                            .unwrap()
                            .send(frame)
                            .await.is_err() {
                                warn!("Failed to send buffer");
                            }
                    }
                }
            }
            Some(frame) = mux_rx.recv() => {
                if let Ok(_) = frame.serialize(&mut raw_buf) {
                    ingress.write_all(&raw_buf).await.expect("Failed to write buffer");
                    ingress.flush().await.expect("Failed to flush buffer");
                };
            }
            Some(res) = join_set.join_next() => {
                match res {
                    Ok(id) => {tx_pool.remove(&id);}
                    Err(e) => {warn!("Something weird happend with an underlying task: {}", e)}
                }
            }
            _ = token.cancelled() => {
                return
            }
        }
    }
}

async fn handle_tcp(
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
    token: CancellationToken,
    destination: SocketAddrV4,
) -> u32 {
    debug!("TCP channel {} opened. Destination: {}", id, destination);
    if let Ok(mut egress) = TcpStream::connect(destination).await {
        loop {
            let mut buf = [0u8; BUFSIZE];
            tokio::select! {
                read_result = egress.read(&mut buf) => {
                    let (ftype, size) =  match read_result {
                        Err(e) if e.kind() == ErrorKind::ConnectionAborted || e.kind() == ErrorKind::ConnectionReset => {
                            debug!("TCP channel {}: Client disconnected. Maybe RST", id);
                            (FrameType::HalfClosed, 0)
                        }
                        Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                            debug!("TCP channel {}: Client disconnected. Maybe FIN", id);
                            (FrameType::HalfClosed, 0)
                        }
                        Err(e) => {
                            warn!("TCP channel {}: Client disconnected. Unexpected error: {}", id, e);
                            (FrameType::HalfClosed, 0)
                        }
                        Ok(nread) if nread == 0 => {
                            debug!("TCP channel {}: Channel closed with EOF", id);
                            (FrameType::HalfClosed, 0)
                        }
                        Ok(nread) => {
                            (FrameType::Data, nread)
                        }
                    };
                    let frame = Frame {
                        header: Header {
                            id: id,
                            ftype: ftype,
                            protocol: Protocol::TCP,
                            size: size as u32,
                            dst: destination,
                        },
                        payload: buf
                    };
                    if tx.send(frame).await.is_err() {
                        error!("TCP channel {}: Failed to send {:?} frame. \
                                Local channel might transit to stale state", id, ftype);
                        break
                    }
                    if ftype == FrameType::HalfClosed {
                        break
                    }
                }
                Some(frame) = rx.recv() => {
                    match frame.header.ftype {
                        FrameType::Data => {
                            match egress.write_all(&frame.payload[..frame.header.size as usize]).await {
                                Ok(_) => {continue}
                                Err(e) => {
                                    warn!("TCP channel {}: Failed to write buffer: {}", id, e);
                                    let frame = Frame {
                                        header: Header {
                                            id: id,
                                            ftype: FrameType::HalfClosed,
                                            protocol: Protocol::TCP,
                                            size: 0,
                                            dst: destination,
                                        },
                                        payload: buf
                                    };
                                    if tx.send(frame).await.is_err() {
                                        error!("TCP channel {}: Failed to send HalfClosed frame. \
                                                Local channel might transit to stale state", id);
                                    }
                                }
                            }
                        },
                        FrameType::HalfClosed => {
                            debug!("TCP channel {}: HalfClosed received", id);
                        }
                        FrameType::Rst => {
                            debug!("TCP channel {}: Rst received. Doesn't expect this tho", id);
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
        warn!("TCP channel {}: Failed to connect to destination", id);
    }
    debug!("TCP channel {}: Channel closed", id);

    return id;
}

async fn handle_dns(
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
    dns_resolvers: Arc<Vec<SocketAddrV4>>,
) -> u32 {
    debug!("DNS channel {} opened", id);
    for resolver in dns_resolvers.iter() {
        if let Ok(egress) = UdpSocket::bind(format!("0.0.0.0:0")).await
            && let Ok(_) = egress.connect(resolver).await
            && let Some(frame) = rx.recv().await
            && let Ok(_) = egress
                .send(&frame.payload[..frame.header.size as usize])
                .await
        {
            debug!("DNS channel {}: Sent some bytes to {}", id, resolver);
            let mut buf = [0u8; BUFSIZE];
            match egress.recv_from(&mut buf).await {
                Ok((nrecv, _)) => {
                    debug!("DNS channel {}: Received some bytes from resolver", id);
                    let frame = Frame {
                        header: Header {
                            id: id,
                            ftype: FrameType::Data,
                            protocol: Protocol::DNS,
                            size: nrecv as u32,
                            dst: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                        },
                        payload: buf,
                    };
                    if tx.send(frame).await.is_err() {
                        warn!("DNS channel {}: Failed to send frame", id);
                    }
                }
                Err(e) => {
                    warn!("DNS channel {}: Failed to receive frame: {}", id, e)
                }
            }
            break;
        } else {
            warn!("DNS channel {}: Failed to init DNS", id);
        }
    }
    debug!("DNS channel {}: Channel closed", id);

    return id;
}

pub async fn init_remote_proxy(token: CancellationToken) {
    mux(token).await;
}
