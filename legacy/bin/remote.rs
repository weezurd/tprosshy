use log::info;
use tprosshy::{
    init_remote_proxy,
    utils::{self},
};

use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    utils::init_logger(Some("/tmp/tprosshy.log".to_string()));
    let task_tracker = tokio_util::task::TaskTracker::new();
    let token = CancellationToken::new();

    task_tracker.spawn(init_remote_proxy(token));
    task_tracker.close();
    info!("Remote proxy started");
    task_tracker.wait().await;
    info!("Remote proxy finished");
}
