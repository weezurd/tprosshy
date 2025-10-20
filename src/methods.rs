mod nft;

use socket2;
use std::error::Error;
use std::net::SocketAddrV4;

fn get_original_dst(sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>> {
    let dst_addr_v4 = sock_ref
        .original_dst_v4()
        .expect("Failed to get orginal destination")
        .as_socket_ipv4()
        .expect("Failed to convert original destination to ipv4");
    return Ok(dst_addr_v4);
}

pub trait BaseMethod {
    fn setup_fw(
        &self,
        allow_ips: &str,
        tcp_port: u16,
        udp_port: u16,
    ) -> Result<(), Box<dyn Error>>;
    fn restore_fw(&self) -> Result<(), Box<dyn Error>>;
    fn get_original_dst(&self, sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>>;
}

pub fn get_available_method() -> Box<dyn BaseMethod + Send + Sync> {
    Box::new(nft::Method::new())
}
