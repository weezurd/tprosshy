use super::{BaseMethod, MethodError};
use once_cell::sync::Lazy;
use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;
pub struct Method {
    path: String,
}
use crate::get_local_nameserver;

/// nftables ruleset template used to configure transparent proxying.
///
/// The template creates a dedicated `ip` table named `tprosshy` with:
/// - `PREROUTING` and `OUTPUT` NAT chains
/// - A shared `PORTAL` chain that performs redirection
///
/// Placeholders are substituted at runtime:
/// - `{{allow_ips}}` — destination IP range to redirect
/// - `{{tcp_port}}` — local TCP listener port
/// - `{{udp_port}}` — local UDP (DNS) listener port
/// - `{{local_nameserver}}` — system DNS resolver IP
///
/// # Notes
/// - Existing `tprosshy` table is deleted before being recreated.
/// - Only IPv4 is supported.
/// - DNS interception is limited to UDP port 53.
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
add rule ip tprosshy PORTAL meta l4proto udp ip daddr {{local_nameserver}} udp dport 53 redirect to {{udp_port}}
add rule ip tprosshy PORTAL fib daddr type local counter return
"#
    .to_string()
});

impl Method {
    /// Creates a new nftables firewall method.
    ///
    /// This function locates the `nft` binary using the system `PATH` and
    /// stores the resolved path for later use.
    ///
    /// # Panics
    /// - Panics if the `nft` binary cannot be found.
    ///
    /// # Assumptions
    /// - `nftables` is installed and available.
    /// - The running system supports the `ip` nftables family.
    pub fn new() -> Self {
        let path = which::which("nft")
            .expect("Failed to find binary location")
            .to_string_lossy()
            .into_owned();
        Method { path }
    }
}

impl BaseMethod for Method {
    /// Applies nftables rules to enable transparent proxying.
    ///
    /// This method:
    /// - Resolves the local system nameserver
    /// - Renders the nftables ruleset template
    /// - Writes the ruleset to a temporary file
    /// - Applies it using `sudo nft -f`
    ///
    /// # Parameters
    /// - `allow_ips`: Destination IP range to redirect (CIDR or nft-compatible).
    /// - `tcp_port`: Local TCP port to redirect traffic to.
    /// - `udp_port`: Local UDP port for DNS redirection.
    ///
    /// # Returns
    /// - `Ok(())` if the ruleset is successfully applied.
    /// - `Err(MethodError)` if any step fails.
    ///
    /// # Requirements
    /// - Requires root privileges via `sudo`.
    /// - Requires a valid `/etc/resolv.conf` with a resolvable nameserver.
    ///
    /// # Side Effects
    /// - Modifies system-wide nftables state.
    fn setup_fw(&self, allow_ips: &str, tcp_port: u16, udp_port: u16) -> Result<(), MethodError> {
        let local_nameserver = match get_local_nameserver() {
            Some(ns) => ns,
            None => {
                return Err(MethodError::SetupError(String::from(
                    "Failed to get local nameserver",
                )));
            }
        };
        let ruleset = RULESET_TEMPLATE
            .replace("{{allow_ips}}", allow_ips)
            .replace("{{tcp_port}}", &tcp_port.to_string())
            .replace("{{udp_port}}", &udp_port.to_string())
            .replace("{{local_nameserver}}", &local_nameserver);

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

    /// Removes the nftables rules created by this method.
    ///
    /// This deletes the `tprosshy` table from the `ip` nftables family.
    ///
    /// # Returns
    /// - `Ok(())` if the table is successfully deleted.
    /// - `Err(MethodError)` if deletion fails.
    ///
    /// # Requirements
    /// - Requires root privileges via `sudo`.
    ///
    /// # Notes
    /// - Failure to restore the firewall may leave the system in a
    ///   redirected state.
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
