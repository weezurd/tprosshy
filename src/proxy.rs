use log::{error, info, warn};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream, UdpSocket},
};

use crate::{get_original_dst, ssh};
use fast_socks5::client::Socks5Stream;
use tokio::sync::mpsc;
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

    let dns_listener_rx = Arc::new(dns_listener);
    let dns_listener_tx = dns_listener_rx.clone();

    let (tx, rx) = mpsc::channel(1);
    task_tracker.spawn(handle_dns(token.clone(), rx, dns_listener_tx));

    info!(
        "Proxy server started. TCP listener address: {}. DNS listener address: {}. SOCKS5 port: {}",
        tcp_listener.local_addr().unwrap(),
        dns_listener_rx.local_addr().unwrap(),
        socks_port
    );

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_port));
            }
            Ok((buflen, addr)) = dns_listener_rx.recv_from(&mut buf) => {
                if buflen < DNS_BUFSIZE {
                    // task_tracker.spawn(handle_dns(buf, buflen, addr, leaked_dns_listener));
                    if let Err(e) = tx.send(DnsCmd::Query { data: buf[..buflen].to_vec(), addr }).await {
                        warn!("Failed to send DNS command to connection manager: {}", e);
                        continue
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
    Query { data: Vec<u8>, addr: SocketAddr },
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

async fn handle_dns(
    token: CancellationToken,
    mut rx: mpsc::Receiver<DnsCmd>,
    dns_listener_tx: Arc<UdpSocket>,
) {
    let mut stream = match TcpStream::connect(&format!("127.0.0.1:{}", LOCAL_DNS_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to connect to dns server: {}", e);
            token.cancel();
            return;
        }
    };

    while let Some(cmd) = rx.recv().await {
        match cmd {
            DnsCmd::Query { data, addr } => {
                let len = (data.len() as u16).to_be_bytes();
                if stream.write_all(&len).await.is_err() || stream.write_all(&data).await.is_err() {
                    warn!("Failed to make DNS request");
                    token.cancel();
                    let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                    return;
                }

                let mut resp_len_buf = [0u8; 2];
                if let Err(e) = stream.read_exact(&mut resp_len_buf).await {
                    warn!("Failed to read DNS response length: {}", e);
                    token.cancel();
                    let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                    return;
                }
                let resp_len = u16::from_be_bytes(resp_len_buf) as usize;
                let mut resp_buf = vec![0u8; resp_len];
                if let Err(e) = stream.read_exact(&mut resp_buf).await {
                    warn!("Failed to read DNS response: {}", e);
                    token.cancel();
                    let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                    return;
                }

                if let Err(e) = dns_listener_tx.send_to(&resp_buf, addr).await {
                    warn!("Failed to send DNS response back to client: {}", e);
                    token.cancel();
                    return;
                }
            }
        }
    }
}
