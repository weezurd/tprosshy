use std::{error::Error, process::Stdio};

use tokio::process::{Child, Command};

pub fn ssh(
    user: &str,
    host: &str,
    port: u16,
    identity_file: &str,
    remote_command: Option<&str>,
) -> Child {
    let mut cmd = Command::new("ssh");
    cmd.arg("-p");
    cmd.arg(&port.to_string());
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=no");
    cmd.arg("-o");
    cmd.arg("UserKnownHostsFile=/dev/null");
    cmd.arg("-o");
    cmd.arg("LogLevel=ERROR");
    cmd.arg("-o");
    cmd.arg("ControlPath=~/.ssh/cm-%r@%h:%p");
    cmd.arg("-o");
    cmd.arg("ControlMaster=auto");
    cmd.arg("-o");
    cmd.arg("ControlPersist=10m");
    cmd.arg("-i");
    cmd.arg(identity_file);
    match remote_command {
        Some(x) => {
            cmd.arg(&format!("{}@{}", user, host));
            cmd.arg(x);
        }
        None => {
            cmd.arg("-N");
            cmd.arg(&format!("{}@{}", user, host));
        }
    };

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to ssh")
}

pub async fn scp(
    user: &str,
    host: &str,
    port: u16,
    identity_file: &str,
    local_file: &str,
    remote_file: &str,
) -> Result<(), Box<dyn Error>> {
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
            &format!("{}@{}:{}", user, host, remote_file),
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
