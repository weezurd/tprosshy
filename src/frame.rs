use std::net::SocketAddrV4;

use bincode;
use bincode::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
pub enum FrameType {
    Data,
    GoAway,
    Ping,
    Rst,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub enum Protocol {
    TCP,
    UDP,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub struct Header {
    pub id: u32,
    pub ftype: FrameType,
    pub protocol: Protocol,
    pub size: u32,
    pub dst: SocketAddrV4,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub struct Frame {
    pub header: Header,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, bincode::error::EncodeError> {
        bincode::encode_into_slice(&self, buf, bincode::config::standard())
    }
}

impl Header {
    pub fn deserialize(buf: &[u8]) -> Result<(Header, usize), bincode::error::DecodeError> {
        bincode::decode_from_slice(buf, bincode::config::standard())
    }
}
