use log::{info, warn};
use std::net::IpAddr;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::time::sleep;

const RETRY: usize = 10;

/// Spawns an `ssh` process with optional SOCKS and port-forwarding configuration.
///
/// This function starts an SSH connection using OpenSSH’s built-in forwarding
/// mechanisms (`-D` for dynamic/SOCKS forwarding and `-L` for local port forwarding).
/// It relies on SSH ControlMaster to allow connection reuse and status checks.
///
/// If `check_connection` is enabled, the function will poll the control socket
/// using `ssh -O check` until the connection is established or a retry limit
/// is reached.
///
/// # Parameters
/// - `host`: SSH host (as defined in SSH config or `user@host` format).
/// - `socks_port`: Optional local port for SOCKS5 dynamic forwarding (`-D`).
/// - `remote_command`: Optional command to execute on the remote host.
/// - `local_portforwarding`: Optional local port forwarding specification (`-L`).
/// - `check_connection`: Whether to actively wait for the SSH control connection
///   to become ready.
///
/// # Returns
/// - `Ok(Child)` if the SSH process is successfully started (and verified if
///   `check_connection` is true).
/// - `Err(String)` if spawning fails or the connection does not become ready
///   within the retry limit.
///
/// # Notes
/// - Host key checking is disabled.
/// - The SSH process is killed if connection checks fail.
pub(crate) async fn ssh(
    host: &str,
    socks_port: Option<u16>,
    remote_command: Option<String>,
    local_portforwarding: Option<String>,
    check_connection: bool,
) -> Result<Child, String> {
    let control_path = format!("~/.ssh/tprosshy-cm-{}", host.replace('@', "-"));
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "LogLevel=ERROR",
        "-o",
        "ControlMaster=auto",
        "-o",
        "ControlPersist=10m",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        &format!("ControlPath={}", control_path),
    ]);
    if let Some(port) = socks_port {
        cmd.arg("-D").arg(port.to_string());
    }
    if let Some(pf) = local_portforwarding {
        cmd.arg("-L").arg(pf);
    }
    cmd.arg(host);
    if let Some(rc) = remote_command {
        cmd.arg(rc);
    }

    info!("Spawning command: {:?}", cmd);
    let mut ssh_proc = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn() {
        Ok(x) => x,
        Err(e) => {
            return Err(format!("Failed to spawn ssh_proc process: {}", e));
        }
    };

    if check_connection {
        for i in 0..RETRY {
            sleep(Duration::from_millis(3000)).await;
            match Command::new("ssh")
                .arg("-o")
                .arg(format!("ControlPath={}", control_path))
                .arg("-O")
                .arg("check")
                .arg(host)
                .output()
                .await
            {
                Ok(x) => {
                    if x.status.success() {
                        return Ok(ssh_proc);
                    }
                    warn!("Waiting for SSH connection... (attempt {})", i + 1);
                }
                Err(e) => {
                    warn!("Failed to check ssh connection: {} (attempt {})", e, i + 1)
                }
            }
        }

        let _ = ssh_proc.kill().await;
        Err(format!("SSH connection timed out after {} attempts", RETRY))
    } else {
        return Ok(ssh_proc);
    }
}

/// Retrieves the first DNS nameserver configured on the remote host.
///
/// This function connects to the remote system via SSH and extracts the first
/// `nameserver` entry from `/etc/resolv.conf`. It assumes a Linux-like environment
/// and does not handle systemd-resolved or other resolver abstractions.
///
/// Internally, it spawns a short-lived SSH process, reads a single line from
/// stdout, and then terminates the process.
///
/// # Parameters
/// - `remote_host`: SSH host to query.
///
/// # Returns
/// - `Some(IpAddr)` if a valid nameserver IP is found and parsed.
/// - `None` if the SSH command fails, no nameserver is present, or the output
///   cannot be parsed as an IP address.
///
/// # Caveats
/// - Only the first `nameserver` entry is considered.
/// - IPv4 and IPv6 are supported only if the returned value parses correctly
///   as `IpAddr`.
pub(crate) async fn get_remote_nameserver(remote_host: &str) -> Option<IpAddr> {
    match ssh(
        remote_host,
        None,
        Some(String::from(
            r#"awk '$1 == "nameserver" {print $2; exit}' /etc/resolv.conf"#,
        )),
        None,
        false,
    )
    .await
    {
        Ok(mut ssh_proc) => {
            let mut raw_ip_addr = String::new();
            if let Some(mut child_stdout) = ssh_proc.stdout.take().map(tokio::io::BufReader::new)
                && let Err(e) = child_stdout.read_line(&mut raw_ip_addr).await
            {
                warn!("Failed to read ssh process stdout: {}", e);
                return None;
            }

            if let Err(e) = ssh_proc.kill().await {
                warn!("Failed to kill ssh process: {}", e)
            }

            let ip_str = raw_ip_addr.trim();
            if ip_str.is_empty() {
                warn!("No nameserver found in remote /etc/resolv.conf");
                return None;
            }

            match ip_str.parse::<IpAddr>() {
                Ok(ip) => Some(ip),
                Err(e) => {
                    warn!("Failed to parse remote nameserver IP '{}': {}", ip_str, e);
                    None
                }
            }
        }
        Err(e) => {
            warn!(
                "Failed to init ssh process: {}. Please check if remote server \"{}\" is accessible",
                e, remote_host
            );
            None
        }
    }
}
