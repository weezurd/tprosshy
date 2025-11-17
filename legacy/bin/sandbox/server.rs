use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    println!("Server running on 127.0.0.1:8080");

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("New connection from: {}", addr);

        // spawn a new task per connection
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket).await {
                eprintln!("Error handling client {}: {}", addr, e);
            }
        });
    }
}

async fn handle_client(mut socket: TcpStream) -> Result<(), Box<dyn Error>> {
    let mut buf = [0u8; 1024];

    loop {
        let n = socket.read(&mut buf).await?;

        if n == 0 {
            println!("Connection closed");
            break;
        }

        let n = socket.write(&buf[..n]).await?;
        socket.flush().await?;
        println!("{} bytes written", n);
    }

    Ok(())
}
