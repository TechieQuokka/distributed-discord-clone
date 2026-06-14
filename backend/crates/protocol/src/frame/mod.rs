//! 프레임 헤더 + 길이접두사 프레이밍 (개념: frame). node-wire.md §1-2.

use crate::codec::{DecodeError, Reader, Writer};

pub const PROTOCOL_VERSION: u8 = 1;
/// version(1)+msg_type(2)+flags(1)+trace_id(16)+src_node_id(8).
pub const HEADER_LEN: usize = 28;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

pub const FLAG_COMPRESSED: u8 = 0x01;
pub const FLAG_REQUIRES_ACK: u8 = 0x02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub msg_type: u16,
    pub flags: u8,
    pub trace_id: u128,
    pub src_node_id: u64,
}

impl Header {
    pub fn new(msg_type: u16, src_node_id: u64, trace_id: u128) -> Self {
        Self { version: PROTOCOL_VERSION, msg_type, flags: 0, trace_id, src_node_id }
    }

    pub fn encode(&self, w: &mut Writer) {
        w.u8(self.version);
        w.u16(self.msg_type);
        w.u8(self.flags);
        w.u128(self.trace_id);
        w.u64(self.src_node_id);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let version = r.u8()?;
        if version != PROTOCOL_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let msg_type = r.u16()?;
        let flags = r.u8()?;
        let trace_id = r.u128()?;
        let src_node_id = r.u64()?;
        Ok(Self { version, msg_type, flags, trace_id, src_node_id })
    }
}

/// payload(header+body)를 길이접두사 프레임으로: `[len u32][payload]`.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// 스트림에서 1프레임 분리. 반환: `Some((payload, 소비 바이트))` 또는 `None`(아직 덜 옴).
pub fn read_frame(buf: &[u8]) -> Result<Option<(&[u8], usize)>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[..4].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(DecodeError::FrameTooLarge(len));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    Ok(Some((&buf[4..4 + len], 4 + len)))
}
