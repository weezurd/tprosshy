use clap::Parser;
use log::info;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::net::TcpListener;

use tokio_util::sync::CancellationToken;
use tprosshy::{Args, get_available_net_tool, init_logger, init_proxy, ssh};

#[tokio::main]
async fn main() {
    init_logger(None);
    let args: Args = Args::parse();
    let net_tool = Arc::new(get_available_net_tool());

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    let _ = ssh(&args.host, args.dynamic_port);
    let tcp_listener = TcpListener::bind("0.0.0.0:0")
        .await
        .expect("Failed to bind address");
    let binding_addr = tcp_listener
        .local_addr()
        .expect("Failed to bind address for tcp listener");
    net_tool
        .setup_fw(&args.ip_range, binding_addr.port())
        .expect("Failed to setup firewall");
    task_tracker.spawn(init_proxy(
        net_tool.clone(),
        token.clone(),
        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), args.dynamic_port),
        tcp_listener,
    ));
    task_tracker.close();

    info!(
        "Proxy server started. TCP port: {}. SOCKS5 port: {}",
        binding_addr.port(),
        args.dynamic_port
    );
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received. Attempt to gracefully shutdown...");
    token.cancel();
    task_tracker.wait().await;
    net_tool.restore_fw().expect("Failed to restore firewall");
    info!("Proxy server finished");
}
