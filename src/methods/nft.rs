use super::{BaseMethod, MethodError};
use once_cell::sync::Lazy;
use std::io::Write;
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
    fn setup_fw(&self, allow_ips: &str, tcp_port: u16) -> Result<(), MethodError> {
        let ruleset = RULESET_TEMPLATE
            .replace("{{allow_ips}}", allow_ips)
            .replace("{{tcp_port}}", &tcp_port.to_string());

        if let Ok(mut tmp_file) = NamedTempFile::new_in("/tmp")
            && let Ok(_) = write!(tmp_file, "{}", ruleset)
            && let Some(tmp_file_path) = tmp_file.path().to_str()
        {
            if let Ok(status) = Command::new("sudo")
                .args([&self.path, "-f", tmp_file_path])
                .status()
                && status.success()
            {
                return Ok(());
            } else {
                return Err(MethodError::SetupError(String::from(
                    "Failed to setup nft table",
                )));
            }
        } else {
            return Err(MethodError::SetupError(String::from(
                "Failed to setup temp file for nft table",
            )));
        }
    }

    fn restore_fw(&self) -> Result<(), MethodError> {
        if let Ok(status) = Command::new("sudo")
            .args([&self.path, "delete", "table", "ip", "tprosshy"])
            .status()
            && status.success()
        {
            return Ok(());
        } else {
            return Err(MethodError::RestoreError(String::from(
                "Failed to delete nft table",
            )));
        }
    }
}
