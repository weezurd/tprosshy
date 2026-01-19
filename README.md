**TODO**
- [ ] Tracing to profile / improve performance
- [ ] Optimize DNS proxy
- [ ] Ipv6?

## TCP
- TCP segment size is somewhat unpredictable due to several factors. Ref: https://datatracker.ietf.org/doc/html/rfc9293#name-segmentation. This behavior might need to be considered when implement tcp listener.

## Command snippet
- Run proxy:
```
cargo run -- --host [SSH CONFIG HOST] --ip [CIDR]
```
- Run proxy with tracing:
    - Run proxy with additional flag and option:
    ```
    RUSTFLAGS="--cfg tokio_unstable" cargo run -- --host [SSH CONFIG HOST] --ip [CIDR] --tracing
    ```
    - In another terminal, run `tokio-console`:

## Dev note
- Always flush if underlying IO is buffered
- Don't use `connect` with `Arc<UdpSocket>`. It causes underlying `UdpSocket` to stick to a specific address. Use `send_to` instead.  
