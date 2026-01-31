# TPROSSHY
## About
- `tprosshy` is **another** [rewrite of sshuttle in Rust](https://github.com/sshuttle/sshuttle_rust/tree/main), with following features:
    - It is written in rust, not Python.
    - It talks to ssh using the "ssh -D" socks support, as a result it does not require any additional code on server.
    - **NEW**: DNS support via DNS-over-TCP, which also does not require any additional code on server.
    - It should be considered alpha quality. While it works, there are many missing features.
- Features that are implemented:
    - IPv4
    - TCP support
    - DNS support (with caveats; see below)
    - SOCKS5 support
    - nftables firewall support
- Missing features include, but not limited to:
    - IPv6
    - Other firewalls, such as OSX support
    - UDP support
    - DNS record TTL handling. Current DNS cache is simply a hashmap
    - Other mechanisms to get nameserver from remote `/etc/resolv.conf` in case `awk` is unavailable
    - Daemon support

## Usage
**NOTE:** Replace `cargo run --` with `tprosshy` binary when release build is used:
```
[RUSTFLAGS="--cfg tokio_unstable" RUST_LOG=debug] cargo run -- --host ssh_config_host --ip ip_range [--socks-port socks_port] [--tracing]
```
This creates two ssh tunnels: 
- A "dynamic port forwarding" tunnel (aka SOCKS5 proxy) for tcp. 
- A "local port forwarding" one for dns. 

All tcp and dns traffic are transparently proxied to remote server with the help of firewall. By default ssh is configured with -D 127.0.0.1:1080, the socks address can be changed with the `--socks-port` option.

`--tracing` option could be used with [tokio-console](https://github.com/tokio-rs/console) to collect diagnostic data. `RUSTFLAGS="--cfg tokio_unstable"` is required for this option.

## Dev Note
- Just like `sshuttle`, `tprosshy` is created as a workaround for crappy/unreliable VPN solution. Besides, network programming is both challenging and fun. 
- TCP segment size is somewhat unpredictable due to several factors. Ref: https://datatracker.ietf.org/doc/html/rfc9293#name-segmentation. This behavior might need to be considered when implement tcp listener.
- Always flush if underlying IO is buffered
- Don't use `connect` with `Arc<UdpSocket>`. It causes underlying `UdpSocket` to stick to a specific address. Use `send_to` instead.  