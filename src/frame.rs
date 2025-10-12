use std::net::SocketAddrV4;

use bincode;
use bincode::{Decode, Encode};

use crate::BUFSIZE;

#[derive(Encode, Decode, PartialEq, Debug)]
pub enum FrameType {
    Data,
    HalfClosed,
    Rst,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub enum Protocol {
    TCP,
    DNS,
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
    pub payload: [u8; BUFSIZE],
}

impl Frame {
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, bincode::error::EncodeError> {
        bincode::encode_into_slice(&self, buf, bincode::config::standard())
    }

    pub fn deserialize(buf: &[u8]) -> Result<(Frame, usize), bincode::error::DecodeError> {
        bincode::decode_from_slice(buf, bincode::config::standard())
    }
}
