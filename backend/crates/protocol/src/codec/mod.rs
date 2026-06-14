//! 수제 바이트 코덱 (개념: codec). 빅엔디언 고정폭 + 길이접두사 가변. node-wire.md §3.

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unexpected end of buffer")]
    UnexpectedEof,
    #[error("invalid utf-8 string")]
    InvalidUtf8,
    #[error("unknown tag {0:#x}")]
    UnknownTag(u16),
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(usize),
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
}

/// 바이트 버퍼 작성기.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u128(&mut self, v: u128) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    /// `[len: u32][raw]`.
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
    pub fn string(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// 바이트 버퍼 판독기 (경계 검사 포함).
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::UnexpectedEof);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn u128(&mut self) -> Result<u128, DecodeError> {
        Ok(u128::from_be_bytes(self.take(16)?.try_into().unwrap()))
    }
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }
    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn string(&mut self) -> Result<String, DecodeError> {
        let b = self.bytes()?;
        core::str::from_utf8(b)
            .map(|s| s.to_owned())
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}
