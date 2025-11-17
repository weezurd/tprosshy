use clap::Parser;
use log::info;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use tokio_util::sync::CancellationToken;
use tprosshy::{Args, LOCAL_TCP_PORT, LOCAL_UDP_PORT, get_available_method, init_proxy, ssh};

#[tokio::main]
async fn main() {
    let args: Args = Args::parse();
    let arc_m = Arc::new(get_available_method());
    arc_m
        .setup_fw(&args.ip_range, LOCAL_TCP_PORT, LOCAL_UDP_PORT)
        .expect("Failed to setup firewall");

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    let _ = ssh(&args.host, "/tmp/remote", args.dynamic_port);
    task_tracker.spawn(init_proxy(
        arc_m.clone(),
        token.clone(),
        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), args.dynamic_port),
    ));
    task_tracker.close();

    info!("Local proxy started");
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received. Attempt to gracefully shutdown.");
    token.cancel();
    task_tracker.wait().await;
    arc_m.restore_fw().expect("Failed to restore firewall");
    info!("Local proxy finished");
}
