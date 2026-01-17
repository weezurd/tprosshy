use log::{error, info, warn};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::{
    io::{AsyncReadExt, copy_bidirectional},
    net::{TcpListener, TcpStream, UdpSocket},
};

use crate::{get_original_dst, ssh, utils::DNS_BUFSIZE};
use domain::base::Message;
use fast_socks5::client::Socks5Stream;
use tokio_util::sync::CancellationToken;

use domain::base::MessageBuilder;
use domain::base::iana::Rcode;
use domain::rdata::rfc1035::A;

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

    loop {
        tokio::select! {
            Ok((ingress, _)) = tcp_listener.accept() => {
                task_tracker.spawn(handle_tcp(ingress, token.clone(), socks_port));
            }
            Ok((buflen, addr)) = dns_listener.recv_from(&mut buf) => {
                if buflen < DNS_BUFSIZE {
                    handle_dns(buf, buflen, addr, leaked_remote_host, &dns_listener).await;
                } else {
                    warn!("Skipped big DNS request.");
                }
            }
            _ = token.cancelled() => {
                task_tracker.close();
                task_tracker.wait().await;
                ssh_proc.kill().await.expect("Failed to kill ssh process");
                break
            }
        }
    }
}

fn forge_dns_response(request_bytes: &[u8], resolved_ip_addr: &str) -> Vec<u8> {
    // 1. Parse the incoming request so we can mirror the Header and Question
    let request = Message::from_octets(request_bytes).expect("Invalid request");
    let header = request.header();

    // 2. Initialize a MessageBuilder with a buffer
    // Use the ID from the request so the client accepts the response
    let mut target = MessageBuilder::new_vec();

    // 3. Set Header flags: Response=true, RecursionDesired=mirrored, Authoritative=true
    let header_builder = target.header_mut();
    header_builder.set_id(header.id());
    header_builder.set_qr(true);
    header_builder.set_rd(header.rd());
    header_builder.set_ra(true);
    header_builder.set_rcode(Rcode::NOERROR);

    // 4. Mirror the Question section
    // A DNS response MUST contain the question it is answering
    let mut question_builder = target.question();
    for q in request.question() {
        question_builder.push(q.expect("Valid question")).unwrap();
    }

    // 5. Build the Answer section
    let mut answer_builder = question_builder.answer();
    let forged_ip: Ipv4Addr = resolved_ip_addr.parse().unwrap();
    let ttl = 60; // 1 minute
    for q in request.question() {
        let q_entry = q.unwrap();
        answer_builder
            .push((q_entry.qname(), ttl, A::new(forged_ip)))
            .unwrap();
    }
    info!("Done forging dns response");

    // 6. Finish and return the bytes
    answer_builder.finish().into()
}

async fn handle_dns(
    buf: [u8; DNS_BUFSIZE],
    buflen: usize,
    addr: SocketAddr,
    remote_host: &str,
    dns_listener: &UdpSocket,
) {
    let msg = Message::from_octets(&buf[..buflen]).expect("Invalid DNS packet");
    for (i, question) in msg.question().enumerate() {
        if i != 0 {
            warn!(
                "Received multiple questions in a single DNS query. This behavior is unsupported."
            );
            break;
        }

        if let Ok(q) = question {
            let target_domain = q.qname();
            info!(
                "DNS request received. Domain: {}. Client address: {}",
                target_domain, addr
            );
            match ssh(
                remote_host,
                None,
                Some(format!("getent hosts {}", target_domain)),
            )
            .stdout
            {
                Some(mut out) => {
                    let mut raw_response = vec![];
                    info!("Reading stdout");
                    if let Ok(len) = out.read_to_end(&mut raw_response).await {
                        if let Ok(resp) = String::from_utf8(raw_response[..len].to_vec()) {
                            let ip_addr = resp.split(" ").collect::<Vec<&str>>()[0];
                            info!("Domain: {}. IP address: {}", target_domain, ip_addr);
                            let dns_response = forge_dns_response(&buf[..buflen], ip_addr);
                            match dns_listener.send_to(&dns_response, addr).await {
                                Ok(n) => {
                                    info!("Send {} bytes to dns client", n)
                                }
                                Err(e) => {
                                    warn!("Failed to send dns response: {}", e)
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        info!("Done dns");
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
    if let Ok(mut remote) = Socks5Stream::connect(
        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), socks_port),
        dst.ip().to_string(),
        dst.port(),
        fast_socks5::client::Config::default(),
    )
    .await
    {
        info!("New connection opened. Destination: {}", dst);
        tokio::select! {
            _ = copy_bidirectional(&mut stream, &mut remote) => {}
            _ = token.cancelled() => {
                return
            }
        }
    } else {
        warn!("Failed to init connection to socks5 server");
    }
    info!("Connection closed.");
}
