**TODO**
- [ ] Tracing to profile / improve performance

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

## Dev note
- Always flush if underlying IO is buffered
- Don't use `connect` with `Arc<UdpSocket>`. It causes underlying `UdpSocket` to stick to a specific address. Use `send_to` instead.  
