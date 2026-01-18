use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};
use tokio::{
    io::{AsyncReadExt, copy_bidirectional},
    net::{TcpListener, TcpStream, UdpSocket},
};

use crate::{get_original_dst, ssh, utils::DNS_BUFSIZE};
use domain::{base::Message, rdata::Aaaa};
use fast_socks5::client::Socks5Stream;
use tokio_util::sync::CancellationToken;

use domain::base::MessageBuilder;
use domain::base::iana::Rcode;
use domain::rdata::rfc1035::A;

static DNS_SERVFAIL: Lazy<Vec<u8>> = Lazy::new(|| {
    let mut builder = MessageBuilder::new_vec();
    builder.header_mut().set_qr(true);
    builder.header_mut().set_rcode(Rcode::SERVFAIL);
    return builder.finish().into();
});

pub async fn init_proxy(
    token: CancellationToken,
    tcp_listener: TcpListener,
    dns_listener: UdpSocket,
    socks_port: u16,
    remote_host: String,
) {
    let task_tracker = tokio_util::task::TaskTracker::new();
    let mut buf = [0u8; DNS_BUFSIZE];
    let mut ssh_proc = ssh(&remote_host, Some(socks_port), None);
    let leaked_remote_host = remote_host.leak();
    let leaked_dns_listener = Box::leak(Box::new(dns_listener));

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_port));
            }
            Ok((buflen, addr)) = leaked_dns_listener.recv_from(&mut buf) => {
                if buflen < DNS_BUFSIZE {
                    task_tracker.spawn(handle_dns(buf, buflen, addr, leaked_remote_host, leaked_dns_listener));
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

fn forge_dns_response(
    dns_request: Message<&[u8]>,
    hostmap: &HashMap<String, Vec<String>>,
) -> Vec<u8> {
    let mut builder = MessageBuilder::new_vec();

    // Header
    let header = builder.header_mut();
    header.set_id(dns_request.header().id());
    header.set_qr(true);
    header.set_rd(dns_request.header().rd());
    header.set_ra(true);
    header.set_rcode(Rcode::NOERROR);

    // Questions (REQUIRED)
    let mut question_builder = builder.question();
    for q in dns_request.question() {
        if let Ok(q_entry) = q {
            if question_builder.push(q_entry).is_err() {
                question_builder.header_mut().set_rcode(Rcode::SERVFAIL);
                return question_builder.finish().into();
            }
        }
    }

    // Answers
    let mut answer_builder = question_builder.answer();
    let ttl = 60;

    for q in dns_request.question() {
        if let Ok(q_entry) = q
            && let Some(ips) = hostmap.get(&q_entry.qname().to_string())
        {
            for ip_str in ips {
                let result = if let Ok(ipv4) = ip_str.parse::<std::net::Ipv4Addr>() {
                    answer_builder.push((q_entry.qname(), ttl, A::new(ipv4)))
                } else if let Ok(ipv6) = ip_str.parse::<std::net::Ipv6Addr>() {
                    answer_builder.push((q_entry.qname(), ttl, Aaaa::new(ipv6)))
                } else {
                    continue;
                };

                if result.is_err() {
                    answer_builder.header_mut().set_rcode(Rcode::SERVFAIL);
                    return answer_builder.finish().into();
                }
            }
        } else {
            answer_builder.header_mut().set_rcode(Rcode::NXDOMAIN);
        }
    }

    answer_builder.finish().into()
}

async fn handle_dns(
    buf: [u8; DNS_BUFSIZE],
    buflen: usize,
    addr: SocketAddr,
    remote_host: &str,
    dns_socket: &UdpSocket,
) {
    let dns_request = match Message::from_octets(&buf[..buflen]) {
        Ok(msg) => msg,
        Err(e) => {
            warn!("Failed to parse dns request: {}", e);
            if let Err(e) = dns_socket.send_to(&DNS_SERVFAIL, addr).await {
                warn!("Failed to send dns response: {}", e)
            }
            return;
        }
    };

    let request_domains: Vec<String> = dns_request
        .question()
        .filter(|x| !x.is_err())
        .map(|x| x.unwrap().qname().clone().to_string())
        .collect();

    if request_domains.len() == 0 {
        warn!("Failed to extract request domains from question section");
        return;
    }

    info!(
        "Command: {}",
        format!("getent hosts {}", request_domains.join(" "))
    );
    let mut child_stdout = match ssh(
        remote_host,
        None,
        Some(format!("getent hosts {}", request_domains.join(" "))),
    )
    .stdout
    {
        Some(x) => x,
        None => {
            warn!("Failed to get ssh process stdout");
            if let Err(e) = dns_socket.send_to(&DNS_SERVFAIL, addr).await {
                warn!("Failed to send dns response: {}", e)
            }
            return;
        }
    };

    let mut raw_response = vec![];
    if let Ok(len) = child_stdout.read_to_end(&mut raw_response).await
        && let Ok(resp_str) = String::from_utf8(raw_response[..len].to_vec())
    {
        let mut hostmap = HashMap::new();
        for line in resp_str.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();

            if parts.len() >= 2 {
                let ip = parts[0].to_string();

                // Everything from index 1 onwards is a valid hostname/alias for this IP
                for i in 1..parts.len() {
                    let hostname = parts[i].to_string();
                    hostmap
                        .entry(hostname)
                        .or_insert_with(Vec::new)
                        .push(ip.clone());
                }
            }
        }

        let dns_response = forge_dns_response(dns_request, &hostmap);
        if let Err(e) = dns_socket.send_to(&dns_response, addr).await {
            warn!("Failed to send dns response: {}", e)
        }
    } else {
        warn!("Failed to read ssh process stdout")
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
