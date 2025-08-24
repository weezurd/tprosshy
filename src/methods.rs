mod nft;

use libc::{SOL_IP, c_int, sockaddr_in, socklen_t};
use std::error::Error;
use std::fmt::Write as FmtWrite;
use std::io;

const SO_ORIGINAL_DST: c_int = 80;
fn get_original_dst(fd: i32) -> Result<String, Box<dyn Error>> {
    let mut addr: sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<sockaddr_in>() as socklen_t;

    let ret = unsafe {
        libc::getsockopt(
            fd,
            SOL_IP,
            SO_ORIGINAL_DST,
            &mut addr as *mut _ as *mut _,
            &mut len,
        )
    };

    if ret != 0 {
        return Err(Box::new(io::Error::last_os_error()));
    }

    let ip_bytes = addr.sin_addr.s_addr.to_ne_bytes(); // In native-endian
    let ip = format!(
        "{}.{}.{}.{}",
        ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]
    );

    let port = u16::from_be(addr.sin_port); // Port is big-endian

    let mut output = String::new();
    write!(&mut output, "{}:{}", ip, port)?;

    Ok(output)
}

pub trait BaseMethod {
    fn setup_fw(&self, allow_ips: &str, local_server_port: u16) -> Result<(), Box<dyn Error>>;
    fn restore_fw(&self) -> Result<(), Box<dyn Error>>;
    fn get_original_dst(&self, fd: i32) -> Result<String, Box<dyn Error>>;
}

pub fn get_available_method() -> Box<dyn BaseMethod + Send + Sync> {
    Box::new(nft::Method::new())
}
