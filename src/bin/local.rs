use clap::Parser;
use log::{debug, error, info, warn};
use std::{
    collections::HashMap,
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::mpsc::{Receiver, Sender},
};

use tokio_util::sync::CancellationToken;
use tprosshy::{
    BUFSIZE, BaseMethod, Frame, FrameType, Header, LOCAL_TCP_PORT, LOCAL_UDP_PORT, MAX_CHANNEL,
    Protocol, get_available_method, scp, ssh,
    utils::{self, IOWrapper},
};

/// Transparent proxy over ssh. Local proxy.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// Remote user
    #[arg(short, long)]
    user: String,

    /// Remote host. Only IPv4 is supported for now.
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

async fn mux(
    mut egress: IOWrapper,
    mut tx_pool: HashMap<u32, tokio::sync::mpsc::Sender<Frame>>,
    mut rx: Receiver<Frame>,
    token: CancellationToken,
) {
    let mut raw_buf = [0u8; size_of::<Frame>()];

    info!("Start demuxing task");
    loop {
        tokio::select! {
            Ok(_) = egress.read_exact(&mut raw_buf) => {
                let (frame, _) = Frame::deserialize(&raw_buf).expect("Failed to deserialize raw buffer");
                // Unwrap guaranteed to work because of how `tx_pool` is constructed.
                tx_pool.get_mut(&frame.header.id).unwrap().send(frame).await.expect("Failed to send frame");
            }
            Some(frame) = rx.recv() => {
                frame.serialize(&mut raw_buf).expect("Failed to serialize frame");
                egress.write_all(&raw_buf).await.expect("Failed to write buffer");
                egress.flush().await.expect("Failed to flush buffer");
            }
            _ = token.cancelled() => {
                let frame = Frame {
                    header: Header {
                        id: 0,
                        ftype: FrameType::Rst,
                        protocol: Protocol::TCP,
                        size: 0,
                        dst: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                    },
                    payload: [0u8; BUFSIZE],
                };
                frame.serialize(&mut raw_buf).expect("Failed to serialize frame");
                egress.write_all(&raw_buf).await.expect("Failed to write buffer");
                egress.flush().await.expect("Failed to flush buffer");
                debug!("RST sent!");
                return
            }
        }
    }
}

async fn handle_tcp(
    mut ingress: TcpStream,
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
    original_dst: SocketAddrV4,
    token: CancellationToken,
) -> (u32, Receiver<Frame>) {
    debug!("TCP channel {} opened", id);
    loop {
        let mut buf = [0; BUFSIZE];
        tokio::select! {
            read_result = ingress.read(&mut buf) => {
                match read_result {
                    Err(e) if e.kind() == ErrorKind::ConnectionAborted || e.kind() == ErrorKind::ConnectionReset => {
                        debug!("TCP channel {}: Client disconnected. Maybe RST", id);
                    }
                    Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                        debug!("TCP channel {}: Client disconnected. Maybe FIN", id);
                    }
                    Err(e) => {
                        warn!("TCP channel {}: Client disconnected. Unexpected error: {}", id, e);
                    }
                    Ok(nread) if nread == 0 => {
                        debug!("TCP channel {}: Channel closed with EOF", id);
                    }
                    Ok(nread) => {
                        let frame = Frame {
                            header: Header {
                                id: id,
                                ftype: FrameType::Data,
                                protocol: Protocol::TCP,
                                size: nread as u32,
                                dst: original_dst,
                            },
                            payload: buf
                        };
                        if tx.send(frame).await.is_err() {
                            warn!("TCP channel {}: Failed to send frame", id);
                            break
                        }
                        continue
                    }
                }
                let frame = Frame {
                    header: Header {
                        id: id,
                        ftype: FrameType::HalfClosed,
                        protocol: Protocol::TCP,
                        size: 0,
                        dst: original_dst,
                    },
                    payload: buf
                };
                if tx.send(frame).await.is_err() {
                    error!("TCP channel {}: Failed to send HalfClosed frame. \
                            Remote channel might transit to stale state", id);
                }
                break
            }
            Some(frame) = rx.recv() => {
                match frame.header.ftype {
                    FrameType::Data => {
                        match ingress.write_all(&frame.payload[..frame.header.size as usize]).await {
                            Ok(_) => {continue}
                            Err(e) => {
                                warn!("TCP channel {}: Failed to write buffer: {}", id, e);
                                let frame = Frame {
                                    header: Header {
                                        id: id,
                                        ftype: FrameType::HalfClosed,
                                        protocol: Protocol::TCP,
                                        size: 0,
                                        dst: original_dst,
                                    },
                                    payload: buf
                                };
                                if tx.send(frame).await.is_err() {
                                    error!("TCP channel {}: Failed to send HalfClosed frame. \
                                            Remote channel might transit to stale state", id);
                                    break
                                }
                            }
                        }
                    },
                    FrameType::HalfClosed => {
                        debug!("TCP channel {}: HalfClosed received", id);
                        break
                    }
                    FrameType::Rst => {
                        debug!("TCP channel {}: Rst received. Doesn't expect this tho", id);
                        break
                    }
                }
            }
            _ = token.cancelled() => {
                break
            }
        }
    }
    debug!("TCP channel {}: Channel closed", id);

    return (id, rx);
}

async fn handle_dns(
    ingress: Arc<UdpSocket>,
    tx: Sender<Frame>,
    mut rx: Receiver<Frame>,
    id: u32,
) -> (u32, Receiver<Frame>) {
    debug!("DNS channel {} opened", id);
    let mut buf = [0u8; BUFSIZE];
    match ingress.try_recv_from(&mut buf) {
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
                        if ingress
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
            warn!("DNS channel: Unexpected error {}", e);
        }
    }

    debug!("DNS channel {}: Channel closed", id);
    return (id, rx);
}

async fn init_local_proxy(
    method: Arc<Box<dyn BaseMethod + Send + Sync>>,
    args: Args,
    token: CancellationToken,
) {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", LOCAL_TCP_PORT))
        .await
        .expect("Failed to bind address");
    let udp_listener = UdpSocket::bind(format!("0.0.0.0:{}", LOCAL_UDP_PORT))
        .await
        .expect("Failed to bind address");
    let udp_listener_guard = Arc::new(udp_listener);

    let task_tracker = tokio_util::task::TaskTracker::new();
    let mut join_set = tokio::task::JoinSet::new();
    let mut tx_pool = HashMap::new();
    let mut rx_pool = vec![];
    for cid in 0..MAX_CHANNEL {
        let (_tx, _rx) = tokio::sync::mpsc::channel::<Frame>(1);
        tx_pool.insert(cid, _tx);
        rx_pool.push((cid, _rx));
    }

    let mut proc = ssh(
        &args.user,
        &args.host,
        args.port,
        &args.identity_file,
        Some(&format!("/tmp/remote")),
    );
    let ssh_in = proc.stdin.take().expect("Failed to acquire stdin");
    let ssh_out = proc.stdout.take().expect("Failed to acquire stdout");
    let egress = utils::IOWrapper {
        tx: utils::Tx::Child(ssh_in),
        rx: utils::Rx::Child(ssh_out),
    };

    let (mux_tx, mux_rx) = tokio::sync::mpsc::channel::<Frame>(1);
    task_tracker.spawn(mux(egress, tx_pool, mux_rx, token.clone()));

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                let orginal_dst = method
                    .get_original_dst(socket2::SockRef::from(&ingress))
                    .expect("Failed to get orignal destination");
                if let Some((id, rx)) = rx_pool.pop() {
                    join_set.spawn(handle_tcp(ingress, mux_tx.clone(), rx, id, orginal_dst,  token.clone()));
                } else {
                    warn!("Channel exhausted");
                }
            }
            _ = udp_listener_guard.readable() => {
                if let Some((id, rx)) = rx_pool.pop() {
                    task_tracker.spawn(handle_dns(udp_listener_guard.clone(), mux_tx.clone(), rx, id));
                } else {
                    info!("Channel exhahsted");
                }
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
    let args: Args = Args::parse();
    // let _ssh_control_master = ssh(&args.user, &args.host, args.port, &args.identity_file, None);
    utils::init_logger(None);
    scp(
        &args.user,
        &args.host,
        args.port,
        &args.identity_file,
        "target/x86_64-unknown-linux-musl/release/remote",
        "/tmp/remote",
    )
    .await
    .expect("Failed to upload executable to remote");
    let arc_m = Arc::new(get_available_method());
    arc_m
        .setup_fw(&args.ip_range, &args.host, LOCAL_TCP_PORT, LOCAL_UDP_PORT)
        .expect("Failed to setup firewall");

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    task_tracker.spawn(init_local_proxy(arc_m.clone(), args.clone(), token.clone()));
    task_tracker.close();

    info!("Local proxy started");
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received. Attempt to gracefully shutdown.");
    token.cancel();
    task_tracker.wait().await;
    arc_m.restore_fw().expect("Failed to restore firewall");
    info!("Local proxy finished");
}
