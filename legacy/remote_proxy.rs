use std::collections::HashMap;

use crate::{
    frame::{Frame, FrameType, Protocol},
    mux::{StreamOrAddr, handle_dns_remote, handle_tcp},
    utils::{self},
};
use log::{info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
                                        StreamOrAddr::Addr(frame.header.dst),
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
                                        frame.header.dst,
                                        token.clone(),
                                    ));
                                }
                                Protocol::DNS => {
                                    join_set.spawn(handle_dns_remote(
                                        mux_tx.clone(),
                                        rx,
                                        frame.header.id,
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
                    Ok((id, _)) => {tx_pool.remove(&id);}
                    Err(e) => {warn!("Something weird happend with an underlying task: {}", e)}
                }
            }
            _ = token.cancelled() => {
                return
            }
        }
    }
}

pub async fn init_remote_proxy(token: CancellationToken) {
    mux(token).await;
}
