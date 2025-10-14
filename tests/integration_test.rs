use clap::Parser;
use log::info;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tprosshy::{
    Args, LOCAL_TCP_PORT, LOCAL_UDP_PORT, get_available_method, init_local_proxy, scp, ssh,
    utils::{self},
};

#[tokio::test]
async fn main() {
    let args: Args = Args::parse();
    utils::init_logger(None);
    scp(
        &args.user,
        &args.host,
        args.port,
        &args.identity_file,
        "target/x86_64-unknown-linux-musl/release/remote",
        "/tmp/remote",
    )
    .await
    .expect("Failed to upload executable to remote");
    let arc_m = Arc::new(get_available_method());
    arc_m
        .setup_fw(&args.ip_range, &args.host, LOCAL_TCP_PORT, LOCAL_UDP_PORT)
        .expect("Failed to setup firewall");

    let token = CancellationToken::new();
    let task_tracker = tokio_util::task::TaskTracker::new();

    task_tracker.spawn(init_local_proxy(
        arc_m.clone(),
        ssh(
            &args.user,
            &args.host,
            args.port,
            &args.identity_file,
            Some(&format!("/tmp/remote")),
        ),
        token.clone(),
    ));
    task_tracker.close();

    info!("Local proxy started");

    info!("Shutdown signal received. Attempt to gracefully shutdown.");
    token.cancel();
    task_tracker.wait().await;
    arc_m.restore_fw().expect("Failed to restore firewall");
    info!("Local proxy finished");
}

async fn request_tcp() {}
async fn request_udp() {}
