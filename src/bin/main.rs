use clap::Parser;
use log::info;
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};

use tokio_util::sync::CancellationToken;
use tprosshy::{Args, get_available_net_tool, init_logger, init_proxy};

#[tokio::main]
async fn main() {
    init_logger(None);
    let args: Args = Args::parse();
    if args.tracing {
        console_subscriber::init();
        info!("Console subscriber initialized");
    }
    let net_tool = Arc::new(get_available_net_tool());

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    let tcp_listener = TcpListener::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    let tcp_binding_addr = tcp_listener
        .local_addr()
        .expect("Failed to bind address for tcp listener");
    let dns_listener = UdpSocket::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    let udp_binding_addr = dns_listener
        .local_addr()
        .expect("Failed to bind address for udp listener");
    net_tool
        .setup_fw(
            &args.ip_range,
            tcp_binding_addr.port(),
            udp_binding_addr.port(),
        )
        .expect("Failed to setup firewall");
    task_tracker.spawn(init_proxy(
        token.clone(),
        tcp_listener,
        dns_listener,
        args.socks_port,
        args.host,
    ));
    task_tracker.close();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received. Attempt to gracefully shutdown...");
            token.cancel();
            task_tracker.wait().await;
            net_tool.restore_fw().expect("Failed to restore firewall");
        }
        _ = task_tracker.wait() => {
            net_tool.restore_fw().expect("Failed to restore firewall");
        }
    };
    info!("Proxy server finished");
}
