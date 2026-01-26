use domain::base::{Message, MessageBuilder, Name, iana::Rcode};
use log::{error, info, warn};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, copy_bidirectional},
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
    let remote_host: &'static str = remote_host.leak();
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
                e, &remote_host
            );
            return;
        }
    };

    let dns_listener_rx = Arc::new(dns_listener);
    let dns_listener_tx = dns_listener_rx.clone();

    let (tx, rx) = mpsc::channel(1);
    task_tracker.spawn(handle_dns(token.clone(), rx, dns_listener_tx, &remote_host));

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

fn create_dns_response(
    request: &Message<&Vec<u8>>,
    qname_str: &str,
    ips: &[String],
) -> Result<Vec<u8>, ()> {
    let mut response = match MessageBuilder::new_vec().start_answer(request, Rcode::NOERROR) {
        Ok(r) => r,
        Err(_) => return Err(()),
    };

    let domain_name = match Name::<Vec<u8>>::from_str(qname_str) {
        Ok(n) => n,
        Err(e) => {
            warn!("Failed to convert qname to domain name: {}", e);
            return Err(());
        }
    };

    for ip_str in ips {
        if let Ok(ip) = ip_str.parse::<IpAddr>() {
            match ip {
                IpAddr::V4(v4) => {
                    let _ = response.push((&domain_name, 300, domain::rdata::A::new(v4)));
                }
                IpAddr::V6(v6) => {
                    let _ = response.push((&domain_name, 300, domain::rdata::Aaaa::new(v6)));
                }
            }
        }
    }

    Ok(response.answer().finish().into())
}

async fn update_record_table(
    remote_host: &str,
    record_table: &mut HashMap<String, Vec<String>>,
    domain_name: &str,
) -> Option<IpAddr> {
    match ssh(
        remote_host,
        None,
        Some(format!("getent ahosts {}", domain_name)),
        None,
    )
    .await
    {
        Ok(mut child) => {
            let stdout = child.stdout.take()?;
            let mut reader = BufReader::new(stdout).lines();

            // Use a HashSet internally to prevent duplicate IPs in the Vec
            let mut unique_ips = HashSet::new();
            let mut first_ip: Option<IpAddr> = None;

            while let Ok(Some(line)) = reader.next_line().await {
                // getent ahosts format: <IP> <SOCK_TYPE> <CANONICAL_NAME>
                if let Some(ip_str) = line.split_whitespace().next() {
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        // Set the primary return value if this is the first valid IP
                        if first_ip.is_none() {
                            first_ip = Some(ip);
                        }
                        // Add to our set to filter out duplicates from different socket types
                        unique_ips.insert(ip_str.to_string());
                    }
                }
            }

            if !unique_ips.is_empty() {
                // Convert HashSet to Vec and update the table
                record_table.insert(remote_host.to_string(), unique_ips.into_iter().collect());
            }

            first_ip
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
    token: CancellationToken,
    mut rx: mpsc::Receiver<DnsCmd>,
    dns_listener_tx: Arc<UdpSocket>,
    remote_host: &str,
) {
    let mut record_table = HashMap::new();
    while let Some(cmd) = rx.recv().await {
        match cmd {
            DnsCmd::Query { data, addr } => {
                let dns_request = match Message::from_octets(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Failed to parse DNS request: {}", e);
                        let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                        continue;
                    }
                };

                let qname = match dns_request
                    .question()
                    .filter(|x| x.is_ok())
                    .map(|x| x.unwrap().qname().to_string())
                    .next()
                {
                    Some(n) => n,
                    None => {
                        warn!("Failed to extract qname from DNS request");
                        let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                        continue;
                    }
                };

                let ips = match record_table.get(&qname) {
                    Some(v) => v,
                    None => {
                        update_record_table(remote_host, &mut record_table, &qname).await;
                        match record_table.get(&qname) {
                            Some(v) => v,
                            None => {
                                warn!("Failed to resolve DNS qname");
                                let _ = dns_listener_tx
                                    .send_to(&create_servfail(&data, data.len()), addr);
                                continue;
                            }
                        };
                        continue;
                    }
                };

                match create_dns_response(&dns_request, &qname, ips) {
                    Ok(v) => {
                        let _ = dns_listener_tx.send_to(&v, addr).await;
                    }
                    Err(()) => {
                        warn!("Failed to create dns response");
                        let _ = dns_listener_tx.send_to(&create_servfail(&data, data.len()), addr);
                        continue;
                    }
                }
            }
        }
    }
}
