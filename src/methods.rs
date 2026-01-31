//! Firewall method abstraction and nftables implementation selection.
//!
//! This module defines a small abstraction layer over firewall backends
//! used by tprosshy to transparently redirect traffic into the SSH tunnel.
//!
//! At the moment, only `nftables` is supported. The design leaves room
//! for additional backends (e.g. iptables, pf, windivert) without
//! leaking backend-specific details into the rest of the codebase.

mod nft;
use std::error::Error;

use std::fmt::{Display, Formatter, Result as FmtResult};

/// Errors returned by firewall setup or teardown.
///
/// This is intentionally simple: errors are already user-facing at this
/// point, so they are stored as strings rather than structured error data.
#[derive(Debug)]
pub(crate) enum MethodError {
    SetupError(String),
    RestoreError(String),
}

impl Display for MethodError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            MethodError::SetupError(e) => write!(f, "{}", e),
            MethodError::RestoreError(e) => write!(f, "{}", e),
        }
    }
}

impl Error for MethodError {}

/// Common interface for firewall backends.
///
/// A `BaseMethod` implementation is responsible for:
/// - Redirecting outbound TCP traffic to the local SOCKS listener
/// - Redirecting DNS traffic to the local DNS proxy
/// - Cleaning up all state on shutdown
///
/// Implementations **must be idempotent** where possible, since setup or
/// teardown may be retried after partial failure.
pub(crate) trait BaseMethod {
    fn setup_fw(&self, allow_ips: &str, tcp_port: u16, udp_port: u16) -> Result<(), MethodError>;
    fn restore_fw(&self) -> Result<(), MethodError>;
}

pub(crate) fn get_available_net_tool() -> Box<dyn BaseMethod + Send + Sync> {
    Box::new(nft::Method::new())
}
