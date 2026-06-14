//! In-process 전송 stub (개념: stub). 같은 프로세스 노드들을 메모리로 연결.
//! Phase 0 배선 + DST(결정론적 시뮬레이션, D25)에 사용. raw-TCP+mTLS는 Phase 2.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use protocol::NodeMessage;
use tokio::sync::mpsc;

use crate::iface::{Inbound, NodeTransport, TransportError};

/// 인프로세스 스위치보드 — 노드들의 수신 채널을 등록/조회.
#[derive(Clone, Default)]
pub struct Switchboard {
    inner: Arc<Mutex<HashMap<u64, mpsc::Sender<Inbound>>>>,
}

impl Switchboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// 노드 등록 → (전송 핸들, 수신 receiver). buffer = bounded(백프레셔 D27).
    pub fn join(
        &self,
        node_id: u64,
        buffer: usize,
    ) -> (InProcessTransport, mpsc::Receiver<Inbound>) {
        let (tx, rx) = mpsc::channel(buffer);
        self.inner.lock().unwrap().insert(node_id, tx);
        (InProcessTransport { node_id, board: self.clone() }, rx)
    }

    /// 노드 제거 (장애 시뮬레이션 — failure detection 테스트용).
    pub fn leave(&self, node_id: u64) {
        self.inner.lock().unwrap().remove(&node_id);
    }
}

pub struct InProcessTransport {
    node_id: u64,
    board: Switchboard,
}

impl NodeTransport for InProcessTransport {
    fn local_node_id(&self) -> u64 {
        self.node_id
    }

    async fn send(&self, dest: u64, msg: NodeMessage) -> Result<(), TransportError> {
        // 락은 await 전에 해제 (sender만 복제해서 빼냄).
        let sender = {
            let guard = self.board.inner.lock().unwrap();
            guard.get(&dest).cloned()
        };
        match sender {
            Some(tx) => tx
                .send(Inbound { src: self.node_id, msg })
                .await
                .map_err(|_| TransportError::Unreachable(dest)),
            None => Err(TransportError::UnknownNode(dest)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delivers_between_nodes() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 8);
        let (_t2, mut r2) = board.join(2, 8);

        t1.send(2, NodeMessage::Ping).await.unwrap();
        let got = r2.recv().await.unwrap();
        assert_eq!(got, Inbound { src: 1, msg: NodeMessage::Ping });
    }

    #[tokio::test]
    async fn unknown_node_errors() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 8);
        assert_eq!(
            t1.send(99, NodeMessage::Ping).await,
            Err(TransportError::UnknownNode(99))
        );
    }

    #[tokio::test]
    async fn leave_makes_node_unknown() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 8);
        let (_t2, _r2) = board.join(2, 8);
        board.leave(2);
        assert_eq!(
            t1.send(2, NodeMessage::Ping).await,
            Err(TransportError::UnknownNode(2))
        );
    }
}
