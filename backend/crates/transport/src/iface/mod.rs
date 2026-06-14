//! 노드 전송 추상 (개념: iface). stub(in-process)/raw-TCP가 이 뒤에 구현 (D10/P3).

use std::future::Future;

use protocol::NodeMessage;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("unknown destination node {0}")]
    UnknownNode(u64),
    #[error("destination node {0} unreachable")]
    Unreachable(u64),
}

/// 수신 봉투 (보낸 노드 + 메시지).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbound {
    pub src: u64,
    pub msg: NodeMessage,
}

/// 노드↔노드 전송. 코어 로직은 이 trait만 알고 구현(TCP인지 stub인지)은 모른다 (P2).
pub trait NodeTransport: Send + Sync + 'static {
    fn local_node_id(&self) -> u64;

    fn send(
        &self,
        dest: u64,
        msg: NodeMessage,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;
}
