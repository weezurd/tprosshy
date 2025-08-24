use std::{error::Error, process::Stdio};

use tokio::process::{ChildStdin, ChildStdout, Command};

pub fn ssh(
    user: &str,
    host: &str,
    port: u16,
    identity_file: &str,
    remote_command: &str,
) -> std::io::Result<(ChildStdin, ChildStdout)> {
    let child = Command::new("ssh")
        .args([
            "-p",
            &port.to_string(),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
            "-o",
            "ControlPath=~/.ssh/cm-%r@%h:%p",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPersist=10m",
            "-i",
            identity_file,
            &format!("{}@{}", user, host),
            remote_command,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to ssh");

    let stdin = child.stdin.expect("Failed to acquire stdin");
    let stdout = child.stdout.expect("Failed to acquire stdout");

    Ok((stdin, stdout))
}

pub async fn scp(
    user: &str,
    host: &str,
    port: u16,
    identity_file: &str,
    local_file: &str,
    remote_file: &str,
) -> Result<(), Box<dyn Error>> {
    let remote = format!("{}@{}:{}", user, host, remote_file);

    let output = Command::new("scp")
        .args([
            "-P",
            &port.to_string(),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-i",
            identity_file,
            local_file,
            &remote,
        ])
        .output()
        .await
        .expect("Failed to spawn scp process");

    if !output.status.success() {
        return Err(format!(
            "Failed to execute scp: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}
