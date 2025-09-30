# Dev note
- [] Use dnsmasq to test udp proxy

## TCP
- TCP segment size is somewhat unpredictable due to several factors. Ref: https://datatracker.ietf.org/doc/html/rfc9293#name-segmentation. This behavior might need to be considered when implement tcp listener.

## Command snippet
- Switch namespace to `local-server`:
```
sudo nsenter -t $(docker inspect --format '{{.State.Pid}}' local-server) -n /bin/sh -c 'sudo su pdd'
```
- Build `local` and `remote` proxy:
```
cargo build --release --target x86_64-unknown-linux-musl --bin remote && cargo build --release --bin local
```