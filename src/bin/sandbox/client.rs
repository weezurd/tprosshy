use std::error::Error;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let stream = TcpStream::connect("127.0.0.1:8080").await?;
    let task_tracker = TaskTracker::new();
    let token = CancellationToken::new();
    println!("Connected to server.");

    let (mut reader, mut writer) = io::split(stream);

    // spawn a task to handle incoming messages
    let t1 = token.clone();
    task_tracker.spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            tokio::select! {
                read_result = reader.read(&mut buf) => {
                    let n = match read_result {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("Failed to read from server: {}", e);
                            break;
                        }
                    };
                    println!("Received {}", String::from_utf8_lossy(&buf[..n]));
                }
                _ = t1.cancelled() => {
                    break
                }
            }
        }
    });
    task_tracker.close();

    for _ in 0..1 {
        writer.write(b"hello_tcp").await?;
        writer.flush().await?;
        sleep(Duration::from_secs(1)).await;
    }

    token.cancel();
    task_tracker.wait().await;

    Ok(())
}
