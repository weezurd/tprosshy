use std::process::Stdio;
extern crate libc;
use tokio::process::{Child, Command};

pub fn ssh(host: &str, socks_port: Option<u16>, remote_command: Option<String>) -> Child {
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
    if let Some(port) = socks_port {
        cmd.arg("-D");
        cmd.arg(port.to_string());
    }
    if let Some(rc) = remote_command {
        cmd.arg(&rc);
    } else {
        cmd.arg("-N");
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to ssh")
}
