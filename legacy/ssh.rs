use std::{error::Error, process::Stdio};
extern crate libc;
use tokio::process::{Child, Command};

pub fn ssh(host: &str, remote_command: &str) -> Child {
    let mut cmd = Command::new("ssh");
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
    cmd.arg(host);
    cmd.arg(remote_command);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to ssh")
}

pub async fn scp(host: &str, local_file: &str, remote_file: &str) -> Result<(), Box<dyn Error>> {
    let output = Command::new("scp")
        .args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            local_file,
            &format!("{}:{}", host, remote_file),
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
