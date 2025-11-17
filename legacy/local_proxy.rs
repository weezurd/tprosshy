use log::{debug, info, warn};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    process::Child,
    sync::mpsc::Receiver,
};

use crate::{
    BUFSIZE, LOCAL_TCP_PORT, LOCAL_UDP_PORT, MAX_CHANNEL,
    frame::{Frame, FrameType, Header, Protocol},
    methods::BaseMethod,
    mux::{StreamOrAddr, handle_dns_local, handle_tcp},
    utils::{self, IOWrapper},
};
use tokio_util::sync::CancellationToken;

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

pub async fn init_local_proxy(
    method: Arc<Box<dyn BaseMethod + Send + Sync>>,
    mut ssh_proc: Child,
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

    let ssh_in = ssh_proc.stdin.take().expect("Failed to acquire stdin");
    let ssh_out = ssh_proc.stdout.take().expect("Failed to acquire stdout");
    let egress = utils::IOWrapper {
        tx: utils::Tx::Child(ssh_in),
        rx: utils::Rx::Child(ssh_out),
    };

    let (mux_tx, mux_rx) = tokio::sync::mpsc::channel::<Frame>(1);
    task_tracker.spawn(mux(egress, tx_pool, mux_rx, token.clone()));

    loop {
        let mut buf: [u8; 32] = [0u8; 32];
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                let orginal_dst = method
                    .get_original_dst(socket2::SockRef::from(&ingress))
                    .expect("Failed to get orignal destination");
                if let Some((id, rx)) = rx_pool.pop() {
                    join_set.spawn(handle_tcp(StreamOrAddr::Stream(ingress), mux_tx.clone(), rx, id, orginal_dst,  token.clone()));
                } else {
                    warn!("Channel exhausted");
                }
            }
            _ = udp_listener_guard.peek(&mut buf) => {
                if let Some((id, rx)) = rx_pool.pop() {
                    join_set.spawn(handle_dns_local(udp_listener_guard.clone(), mux_tx.clone(), rx, id));
                } else {
                    info!("Channel exhausted");
                }
            }
            Some(res) = join_set.join_next() => {
                match res {
                    Ok((id, rx)) => {rx_pool.push((id, rx));}
                    Err(e) => {warn!("Something weird happend with an underlying task: {}", e)}
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
