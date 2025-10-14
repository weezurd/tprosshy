use std::net::SocketAddrV4;

use bincode;
use bincode::{Decode, Encode};

use crate::BUFSIZE;

#[derive(Encode, Decode, PartialEq, Debug)]
pub(crate) enum FrameType {
    Data,
    HalfClosed,
    Rst,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub(crate) enum Protocol {
    TCP,
    DNS,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub(crate) struct Header {
    pub(crate) id: u32,
    pub(crate) ftype: FrameType,
    pub(crate) protocol: Protocol,
    pub(crate) size: u32,
    pub(crate) dst: SocketAddrV4,
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub(crate) struct Frame {
    pub(crate) header: Header,
    pub(crate) payload: [u8; BUFSIZE],
}

impl Frame {
    pub(crate) fn serialize(&self, buf: &mut [u8]) -> Result<usize, bincode::error::EncodeError> {
        bincode::encode_into_slice(&self, buf, bincode::config::standard())
    }

    pub(crate) fn deserialize(buf: &[u8]) -> Result<(Frame, usize), bincode::error::DecodeError> {
        bincode::decode_from_slice(buf, bincode::config::standard())
    }
}
