use crate::{
    BUFSIZE,
    frame::{Frame, FrameType, Header, Protocol},
    utils::get_system_resolvers,
};
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::{
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::mpsc::{Receiver, Sender},
};
use tokio_util::sync::CancellationToken;
static SYSTEM_RESOLVERS: Lazy<Vec<SocketAddrV4>> = Lazy::new(|| get_system_resolvers());

pub(crate) enum StreamOrAddr {
    Stream(TcpStream),
    Addr(SocketAddrV4),
}

pub(crate) async fn handle_tcp(
    stream_or_addr: StreamOrAddr,
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
    destination: SocketAddrV4,
    token: CancellationToken,
) -> (u32, Receiver<Frame>) {
    let mut stream = match stream_or_addr {
        StreamOrAddr::Stream(s) => s,
        StreamOrAddr::Addr(a) => {
            if let Ok(s) = TcpStream::connect(a).await {
                s
            } else {
                warn!("TCP channel {}: Failed to connect to destination", id);
                return (id, rx);
            }
        }
    };
    debug!("TCP channel {} opened", id);

    loop {
        let mut buf = [0; BUFSIZE];
        tokio::select! {
            read_result = stream.read(&mut buf) => {
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
                            Remote channel might transit to stale state", id, ftype);
                    break
                }
                if ftype == FrameType::HalfClosed {
                    break
                }
            }
            Some(frame) = rx.recv() => {
                match frame.header.ftype {
                    FrameType::Data => {
                        match stream.write_all(&frame.payload[..frame.header.size as usize]).await {
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
                                            Remote channel might transit to stale state", id);
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
    debug!("TCP channel {}: Channel closed", id);

    return (id, rx);
}

pub(crate) async fn handle_dns_local(
    socket: Arc<UdpSocket>,
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
) -> (u32, Receiver<Frame>) {
    debug!("DNS channel {} opened", id);
    let mut buf = [0u8; BUFSIZE];
    match socket.try_recv_from(&mut buf) {
        Ok((nrecv, addr)) => {
            debug!("DNS channel {}: Connected to {}..", id, addr);
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
            match tx.send(frame).await {
                Ok(_) => {
                    if let Some(frame) = rx.recv().await {
                        if socket
                            .send_to(&frame.payload[..frame.header.size as usize], addr)
                            .await
                            .is_err()
                        {
                            warn!("DNS channel {}: Failed to send frame", id);
                        }
                    }
                }
                Err(e) => {
                    warn!("DNS channel {}: Failed to send frame: {}", id, e);
                }
            }
        }
        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {}
        Err(e) => {
            warn!("DNS channel {}: Unexpected error {}", id, e);
        }
    }

    debug!("DNS channel {}: Channel closed", id);
    return (id, rx);
}

pub(crate) async fn handle_dns_remote(
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
) -> (u32, Receiver<Frame>) {
    debug!("DNS channel {} opened", id);
    for resolver in SYSTEM_RESOLVERS.iter() {
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

    return (id, rx);
}
