use log::{error, info, warn};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, copy_bidirectional},
    net::{TcpListener, TcpStream, UdpSocket},
};

use crate::{get_original_dst, ssh};
use fast_socks5::client::Socks5Stream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

const LOCAL_DNS_PORT: u16 = 5353;
const DNS_BUFSIZE: usize = 1024;

pub async fn init_proxy(
    token: CancellationToken,
    tcp_listener: TcpListener,
    dns_listener: UdpSocket,
    socks_port: u16,
    remote_host: String,
) {
    let task_tracker = tokio_util::task::TaskTracker::new();
    let mut buf = [0u8; DNS_BUFSIZE];
    let remote_nameserver = match get_remote_nameserver(&remote_host).await {
        Some(x) => x,
        None => {
            warn!(
                "Failed to get remote nameserver. Please check remote nameserver at /etc/resolv.conf"
            );
            return;
        }
    };

    let mut ssh_proc = match ssh(
        &remote_host,
        Some(socks_port),
        None,
        Some(format!("{}:{}:53", LOCAL_DNS_PORT, remote_nameserver)),
    )
    .await
    {
        Ok(x) => x,
        Err(e) => {
            warn!(
                "Failed to init ssh process: {}. Please check if remote server \"{}\" is accessible",
                e, remote_host
            );
            return;
        }
    };
    let leaked_dns_listener = Box::leak(Box::new(dns_listener));

    let (tx, rx) = mpsc::channel(1);
    task_tracker.spawn(dns_connection_manager(
        rx,
        format!("127.0.0.1:{}", LOCAL_DNS_PORT),
    ));

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_port));
            }
            Ok((buflen, addr)) = leaked_dns_listener.recv_from(&mut buf) => {
                if buflen < DNS_BUFSIZE {
                    // task_tracker.spawn(handle_dns(buf, buflen, addr, leaked_dns_listener));
                    let (resp_tx, resp_rx) = oneshot::channel();
                    if let Err(_) = tx.send(DnsCmd::Query { data: buf[..buflen].to_vec(), resp_tx: resp_tx }).await {
                        warn!("Failed to send DNS command to connection manager");
                        continue
                    }

                    match resp_rx.await {
                        Ok(x) => {
                            if let Err(e) = leaked_dns_listener.send_to(&x, addr).await {
                                warn!("Failed to send UDP response back to client: {}", e);
                            }
                        }
                        Err(e) => {warn!("Failed to received DNS result: {}", e)}
                    }

                } else {
                    warn!("Skipped big DNS request.");
                }
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                if let Err(e) = ssh_proc.kill().await {
                    warn!("Failed to kill ssh process: {}", e)
                }
                break
            }
        }
    }
}

async fn get_remote_nameserver(remote_host: &str) -> Option<IpAddr> {
    match ssh(
        remote_host,
        None,
        Some(String::from(
            r#"awk '$1 == "nameserver" {print $2; exit}' /etc/resolv.conf"#,
        )),
        None,
    )
    .await
    {
        Ok(mut child) => {
            let mut raw_ip_addr = String::new();
            if let Some(mut child_stdout) = child.stdout.take().map(tokio::io::BufReader::new)
                && let Err(e) = child_stdout.read_line(&mut raw_ip_addr).await
            {
                warn!("Failed to read ssh process stdout: {}", e);
                return None;
            }

            if let Err(e) = child.kill().await {
                warn!("Failed to kill ssh process: {}", e)
            }

            let ip_str = raw_ip_addr.trim();
            if ip_str.is_empty() {
                warn!("No nameserver found in remote /etc/resolv.conf");
                return None;
            }

            match ip_str.parse::<IpAddr>() {
                Ok(ip) => Some(ip),
                Err(e) => {
                    warn!("Failed to parse remote nameserver IP '{}': {}", ip_str, e);
                    None
                }
            }
        }
        Err(e) => {
            warn!(
                "Failed to init ssh process: {}. Please check if remote server \"{}\" is accessible",
                e, remote_host
            );
            None
        }
    }
}

async fn handle_dns(
    buf: [u8; DNS_BUFSIZE],
    buflen: usize,
    addr: SocketAddr,
    dns_socket: &UdpSocket,
) {
    let mut tcp_payload = Vec::with_capacity(buflen + 2);
    tcp_payload.extend_from_slice(&(buflen as u16).to_be_bytes());
    tcp_payload.extend_from_slice(&buf[..buflen]);

    let send_servfail = || async {
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

        let _ = dns_socket.send_to(&response, addr).await;
    };

    let stream_result = TcpStream::connect(format!("127.0.0.1:{}", LOCAL_DNS_PORT)).await;
    match stream_result {
        Ok(stream) => {
            if let Err(e) = stream.set_nodelay(true) {
                warn!("Failed to set TCP_NODELAY: {}", e);
            }
            let mut buffered_stream = BufReader::new(stream);
            if let Err(e) = buffered_stream.write_all(&tcp_payload).await {
                warn!("Failed to write to DNS tunnel: {}", e);
                send_servfail().await;
                return;
            }

            let mut len_buf = [0u8; 2];
            if let Err(e) = buffered_stream.read_exact(&mut len_buf).await {
                warn!("Failed to read DNS response length: {}", e);
                send_servfail().await;
                return;
            }

            let mut dns_response = vec![0u8; u16::from_be_bytes(len_buf) as usize];
            if let Err(e) = buffered_stream.read_exact(&mut dns_response).await {
                warn!("Failed to read DNS response body: {}", e);
                send_servfail().await;
                return;
            }

            if let Err(e) = dns_socket.send_to(&dns_response, addr).await {
                warn!("Failed to send UDP response back to client: {}", e);
            }
        }
        Err(e) => {
            warn!(
                "Could not connect to LOCAL_DNS_PORT {}: {}",
                LOCAL_DNS_PORT, e
            );
            send_servfail().await;
        }
    };
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
            warn!("Failed to init connection to socks5 server: {}", e);
        }
    }
    info!("Connection closed.");
}

enum DnsCmd {
    Query {
        data: Vec<u8>,
        resp_tx: oneshot::Sender<Vec<u8>>,
    },
}

async fn dns_connection_manager(mut rx: mpsc::Receiver<DnsCmd>, addr: String) {
    let mut stream: Option<TcpStream> = None;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            DnsCmd::Query { data, resp_tx } => {
                // 1. Ensure connected
                if stream.is_none() {
                    // Internal: Connect implies sending SSH_MSG_CHANNEL_OPEN (The 230ms hit)
                    // We only want to do this ONCE.
                    match TcpStream::connect(&addr).await {
                        Ok(s) => {
                            let _ = s.set_nodelay(true); // Crucial for latency
                            stream = Some(s);
                        }
                        Err(_) => {
                            let _ = resp_tx.send(vec![]); // Fail
                            continue;
                        }
                    }
                }

                if let Some(ref mut s) = stream {
                    // 2. Send Length + Data
                    let len = (data.len() as u16).to_be_bytes();
                    if s.write_all(&len).await.is_err() || s.write_all(&data).await.is_err() {
                        // Connection died, drop it and retry/fail
                        stream = None;
                        // ideally you would retry connection here once
                        continue;
                    }

                    // 3. Read Response
                    // Note: This logic blocks other queries. Ideally, split Read/Write
                    // into two tasks if you want full pipelining.
                    let mut len_buf = [0u8; 2];
                    if s.read_exact(&mut len_buf).await.is_err() {
                        stream = None;
                        continue;
                    }
                    let resp_len = u16::from_be_bytes(len_buf) as usize;
                    let mut resp_buf = vec![0u8; resp_len];
                    if s.read_exact(&mut resp_buf).await.is_err() {
                        stream = None;
                        continue;
                    }

                    info!("DNS result sent");
                    let _ = resp_tx.send(resp_buf);
                }
            }
        }
    }
}
