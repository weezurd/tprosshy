use clap::Parser;
use log::info;

use tokio_util::sync::CancellationToken;
use tprosshy::{Args, init_logger, init_proxy};

#[tokio::main]
async fn main() {
    init_logger(None);
    let args: Args = Args::parse();
    if args.tracing {
        console_subscriber::init();
        info!("Console subscriber initialized");
    }

    let task_tracker = tokio_util::task::TaskTracker::new();
    let token = CancellationToken::new();
    task_tracker.spawn(init_proxy(
        token.clone(),
        args.socks_port,
        args.host.leak(),
        args.ip_range,
    ));
    task_tracker.close();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received. Attempt to gracefully shutdown...");
            token.cancel();
            task_tracker.wait().await;
        }
        _ = task_tracker.wait() => {
        }
    };
    info!("Proxy server finished");
}
