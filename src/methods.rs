mod nft;
use std::error::Error;

use std::fmt::{Display, Formatter, Result as FmtResult};

#[derive(Debug)]
pub enum MethodError {
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

pub trait BaseMethod {
    fn setup_fw(&self, allow_ips: &str, tcp_port: u16) -> Result<(), MethodError>;
    fn restore_fw(&self) -> Result<(), MethodError>;
}

pub fn get_available_net_tool() -> Box<dyn BaseMethod + Send + Sync> {
    Box::new(nft::Method::new())
}
