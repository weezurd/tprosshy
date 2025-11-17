use std::process::Stdio;
extern crate libc;
use tokio::process::{Child, Command};

pub fn ssh(host: &str, remote_command: &str, dynamic_port: u16) -> Child {
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
    cmd.arg("-D");
    cmd.arg(dynamic_port.to_string());
    cmd.arg(host);
    cmd.arg(remote_command);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to ssh")
}
