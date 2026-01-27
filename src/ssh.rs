use log::{info, warn};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::time::sleep;

const RETRY: usize = 10;

pub(crate) async fn ssh(
    host: &str,
    socks_port: Option<u16>,
    remote_command: Option<String>,
    local_portforwarding: Option<String>,
    check_connection: bool,
) -> Result<Child, String> {
    let control_path = format!("/tmp/tprosshy-cm-{}", host.replace('@', "-"));
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
            return Err(format!("Failed to spawn child process: {}", e));
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
