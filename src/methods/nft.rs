use super::{BaseMethod, get_original_dst};
use once_cell::sync::Lazy;
use std::error::Error;
use std::io::Write;
use std::net::SocketAddrV4;
use std::process::Command;
use tempfile::NamedTempFile;
pub struct Method {
    path: String,
}

static RULESET_TEMPLATE: Lazy<String> = Lazy::new(|| {
    r#"table ip tprosshy
delete table ip tprosshy
add table ip tprosshy
add chain ip tprosshy PREROUTING { type nat hook prerouting priority dstnat; policy accept; }
add chain ip tprosshy OUTPUT { type nat hook output priority -100; policy accept; }
add chain ip tprosshy PORTAL

add rule ip tprosshy PREROUTING counter jump PORTAL
add rule ip tprosshy OUTPUT counter jump PORTAL
add rule ip tprosshy PORTAL meta l4proto tcp ip daddr 127.0.0.1 return
add rule ip tprosshy PORTAL meta l4proto tcp ip daddr {{allow_ips}} redirect to {{tcp_port}}
# add rule ip tprosshy PORTAL meta l4proto udp ip daddr 127.0.0.53 udp dport 53 redirect to {{udp_port}}
add rule ip tprosshy PORTAL fib daddr type local counter return
"#
    .to_string()
});

impl Method {
    pub fn new() -> Self {
        let path = which::which("nft")
            .expect("Failed to find binary location")
            .to_string_lossy()
            .into_owned();
        Method { path }
    }
}

impl BaseMethod for Method {
    fn setup_fw(
        &self,
        allow_ips: &str,
        tcp_port: u16,
        udp_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let ruleset = RULESET_TEMPLATE
            .replace("{{allow_ips}}", allow_ips)
            .replace("{{tcp_port}}", &tcp_port.to_string())
            .replace("{{udp_port}}", &udp_port.to_string());

        let mut tmp_file = NamedTempFile::new_in("/tmp")?;
        write!(tmp_file, "{}", ruleset)?;

        let status = Command::new("sudo")
            .arg(&self.path)
            .arg("-f")
            .arg(tmp_file.path())
            .status()?;

        if !status.success() {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "nft command failed",
            )))
        } else {
            Ok(())
        }
    }

    fn restore_fw(&self) -> Result<(), Box<dyn Error>> {
        let status = Command::new("sudo")
            .args([&self.path, "delete", "table", "ip", "tprosshy"])
            .status()?;

        if !status.success() {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "restore command failed",
            )))
        } else {
            Ok(())
        }
    }

    fn get_original_dst(&self, sock_ref: socket2::SockRef) -> Result<SocketAddrV4, Box<dyn Error>> {
        get_original_dst(sock_ref)
    }
}
