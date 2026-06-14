//! TLS 스트림 위 NodeMessage 프레이밍 입출력 (개념: wire). D3 길이접두사.
//!
//! 프레임 = `[len u32 BE][header(28B)+body]`. `protocol`의 인코딩/디코딩 재사용,
//! 여기선 TCP 스트림에서 길이만큼 모아 한 프레임씩 읽는 async 경계 처리만 담당.

use std::io;

use protocol::{MAX_FRAME, NodeMessage};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 메시지 1개를 프레임으로 써서 flush. `src`는 프레임 헤더의 src_node_id (피어 식별).
pub async fn write_msg<W: AsyncWrite + Unpin>(
    w: &mut W,
    src: u64,
    msg: &NodeMessage,
) -> io::Result<()> {
    let frame = msg.encode(src, 0); // trace_id는 후속(D26 분산추적 연동)
    w.write_all(&frame).await?;
    w.flush().await
}

/// 프레임 1개를 읽어 `(src_node_id, NodeMessage)` 반환. 스트림 종료 시 Err(UnexpectedEof).
pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(u64, NodeMessage)> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    let (header, msg) = NodeMessage::decode(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok((header.src_node_id, msg))
}
