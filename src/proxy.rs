use domain::base::{Message, MessageBuilder, iana::Rcode};
use domain::rdata::AllRecordData;
use log::{debug, error, info, warn};
use std::error::Error;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    vec,
};
use tokio::process::Child;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream, UdpSocket},
};

use crate::{get_available_net_tool, get_original_dst, get_remote_nameserver, ssh};
use fast_socks5::client::Socks5Stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const LOCAL_DNS_PORT: u16 = 5353;
const DNS_BUFSIZE: usize = 1024;

enum DnsCmd {
    Query { data: Vec<u8>, addr: SocketAddr },
}

async fn init_ssh_tunnel(
    remote_host: &'static str,
    socks_port: u16,
) -> Result<Child, Box<dyn Error>> {
    let remote_nameserver = match get_remote_nameserver(&remote_host).await {
        Some(x) => x,
        None => {
            return Err(
                "Failed to get remote nameserver. Please check remote nameserver at /etc/resolv.conf".to_string().into(),
            );
        }
    };

    match ssh(
        &remote_host,
        Some(socks_port),
        None,
        Some(format!("{}:{}:53", LOCAL_DNS_PORT, remote_nameserver)),
        true,
    )
    .await
    {
        Ok(x) => Ok(x),
        Err(e) => {
            return Err(
                format!("Failed to init ssh process: {}. Please check if remote server \"{}\" is accessible",
                e, &remote_host).into()
            );
        }
    }
}

pub async fn init_proxy(
    token: CancellationToken,
    socks_port: u16,
    remote_host: &'static str,
    ip_range: String,
) {
    let tcp_listener = match TcpListener::bind("0.0.0.0:0").await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind address: {e}");
            return;
        }
    };

    let tcp_binding_addr = match tcp_listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            error!("Failed to bind address for tcp listener: {e}");
            return;
        }
    };

    let dns_listener = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(socket) => socket,
        Err(e) => {
            error!("Failed to bind address: {e}");
            return;
        }
    };

    let udp_binding_addr = match dns_listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            error!("Failed to bind address for udp listener: {e}");
            return;
        }
    };

    let task_tracker = tokio_util::task::TaskTracker::new();
    let dns_listener_rx = Arc::new(dns_listener);
    let dns_listener_tx = dns_listener_rx.clone();
    let (tx, rx) = mpsc::channel(1);
    task_tracker.spawn(handle_dns(token.clone(), rx, dns_listener_tx));

    let mut ssh_tunnel = match init_ssh_tunnel(&remote_host, socks_port).await {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to init ssh tunnel: {e}");
            return;
        }
    };

    let net_tool = get_available_net_tool();
    if let Err(e) = net_tool.setup_fw(&ip_range, tcp_binding_addr.port(), udp_binding_addr.port()) {
        error!("Failed to setup firewall: {e}");
        return;
    }

    info!(
        "Proxy server started. TCP listener address: {}. DNS listener address: {}. SOCKS5 port: {}",
        tcp_listener.local_addr().unwrap(),
        dns_listener_rx.local_addr().unwrap(),
        socks_port
    );

    let mut buf = [0u8; DNS_BUFSIZE];
    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_port));
            }
            Ok((buflen, addr)) = dns_listener_rx.recv_from(&mut buf) => {
                if buflen < DNS_BUFSIZE {
                    if let Err(e) = tx.send(DnsCmd::Query { data: buf[..buflen].to_vec(), addr }).await {
                        warn!("Failed to send DNS request: {e}");
                        continue
                    }
                } else {
                    warn!("Skipped big DNS request.");
                }
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                if let Err(e) = ssh_tunnel.kill().await {
                    warn!("Failed to kill ssh process: {e}")
                }
                net_tool.restore_fw().expect("Failed to restore firewall");
                break
            }
        }
    }
}

async fn handle_tcp(mut stream: TcpStream, token: CancellationToken, socks_port: u16) {
    let dst = match get_original_dst(socket2::SockRef::from(&stream)) {
        Ok(addr) => addr,
        Err(_) => {
            error!("Failed to get original destination of NAT-ed address");
            return;
        }
    };
    match Socks5Stream::connect(
        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), socks_port),
        dst.ip().to_string(),
        dst.port(),
        fast_socks5::client::Config::default(),
    )
    .await
    {
        Ok(mut remote) => {
            info!("New connection opened. Destination: {}", dst);
            tokio::select! {
                _ = copy_bidirectional(&mut stream, &mut remote) => {}
                _ = token.cancelled() => {
                    return;
                }
            }
        }
        Err(e) => {
            warn!("Failed to init connection to socks5 server: {e}");
        }
    }
    info!("Connection closed.");
}

fn create_servfail(buf: &[u8], buflen: usize) -> Vec<u8> {
    let mut response = Vec::with_capacity(buflen);

    let mut header = [0u8; 12];
    header[0..2].copy_from_slice(&buf[0..2]); // Transaction ID
    header[2] = 0x81; // Flags: Response, OpCode 0, RD 1
    header[3] = 0x82; // Flags: RA 1, RCODE 2 (SERVFAIL)
    header[4..6].copy_from_slice(&buf[4..6]); // Echo the original QDCOUNT

    response.extend_from_slice(&header);

    // Attach the Question Section
    if buflen >= 12 {
        response.extend_from_slice(&buf[12..buflen]);
    }

    return response;
}

fn create_dns_response(
    dns_request: &Message<&Vec<u8>>,
    cached_dns_response: &Message<Vec<u8>>,
) -> Result<Vec<u8>, ()> {
    let mut response = match MessageBuilder::new_vec().start_answer(dns_request, Rcode::NOERROR) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to create DNS response: {e}");
            return Err(());
        }
    };

    // INTERNAL MECHANIC: The iterator yields a "Parser" view.
    // We must explicitly parse the RData into a concrete type (AllRecordData)
    // so the builder knows how to re-serialize it.
    // `AllRecordData` is an enum covering all standard types (A, AAAA, MX, etc.).
    let records = cached_dns_response
        .answer()
        .into_iter()
        .flat_map(|it| it)
        .filter(|x| x.is_ok())
        .map(|x| x.unwrap().into_record::<AllRecordData<_, _>>())
        .filter(|x| x.is_ok())
        .map(|x| x.unwrap())
        .filter(|x| x.is_some())
        .map(|x| x.unwrap());

    for r in records {
        debug!("Cached record: {}", r);
        let _ = response.push(r);
    }

    Ok(response.answer().finish().into())
}

async fn update_record_table(
    record_table: &mut HashMap<(String, String), Message<Vec<u8>>>,
    qid: (String, String),
    data: &[u8],
) {
    let mut stream = match TcpStream::connect(format!("localhost:{}", LOCAL_DNS_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to connect to DNS server: {e}");
            return;
        }
    };

    let mut tcp_payload = Vec::with_capacity(data.len() + 2);
    tcp_payload.extend_from_slice(&(data.len() as u16).to_be_bytes());
    tcp_payload.extend_from_slice(&data);
    if let Err(e) = stream.write_all(&tcp_payload).await {
        warn!("Failed to make DNS request: {e}");
        return;
    }

    let mut len_buf = [0u8; 2];
    if let Err(e) = stream.read_exact(&mut len_buf).await {
        warn!("Failed to read DNS response length: {e}");
        return;
    }

    let resp_len = u16::from_be_bytes(len_buf) as usize;
    let mut resp_buf = vec![0u8; resp_len];
    if let Err(e) = stream.read_exact(&mut resp_buf).await {
        warn!("Failed to read DNS response: {e}");
        return;
    }

    match Message::from_octets(resp_buf) {
        Ok(m) => record_table.insert(qid.clone(), m),
        Err(e) => {
            warn!("Failed to parse DNS response: {e}");
            return;
        }
    };
}

async fn handle_dns(
    token: CancellationToken,
    mut rx: mpsc::Receiver<DnsCmd>,
    dns_listener_tx: Arc<UdpSocket>,
) {
    let mut record_table = HashMap::new();
    loop {
        tokio::select! {
            Some(DnsCmd::Query { data, addr }) = rx.recv() => {
                let dns_request = match Message::from_octets(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Failed to parse DNS request: {e}");
                        if let Err(e) = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr).await {
                            warn!("Failed to send DNS response: {e}")
                        };
                        continue;
                    }
                };

                let qid = match dns_request
                    .question()
                    .filter(|x| x.is_ok())
                    .map(|x| x.unwrap())
                    .next()
                {
                    Some(x) => (x.qname().to_string(), x.qtype().to_string()),
                    None => {
                        warn!("Failed to extract qtype from DNS request");
                        if let Err(e) = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr).await {
                            warn!("Failed to send DNS response: {e}")
                        };
                        continue;
                    }
                };


                let answer = match record_table.get(&qid) {
                    Some(v) => v,
                    None => {
                        update_record_table(&mut record_table, qid.clone(), &data).await;
                        match record_table.get(&qid) {
                            Some(v) => v,
                            None => {
                                warn!("Failed to resolve DNS qname: {}", &qid.0);
                                if let Err(e) = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr).await {
                                    warn!("Failed to send DNS response: {e}")
                                };
                                continue;
                            }
                        }
                    }
                };

                match create_dns_response(&dns_request, answer) {
                    Ok(v) => {
                        if let Err(e) = dns_listener_tx.send_to(&v, addr).await {
                            warn!("Failed to send DNS response: {e}")
                        };
                    }
                    Err(()) => {
                        warn!("Failed to create DNS response")
                    }
                }

            }
            _ = token.cancelled() => {
                return;
            }
        }
    }
}
