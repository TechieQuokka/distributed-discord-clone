//! 액터 메일박스(주소) (개념: mailbox).
//! bounded mpsc → 백프레셔 (D27). 주소는 복제 가능(여러 송신자).

use tokio::sync::mpsc;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    #[error("mailbox closed (actor stopped)")]
    Closed,
    #[error("mailbox full (backpressure)")]
    Full,
}

/// 액터로 메시지를 보내는 주소. 내부적으로 bounded `mpsc::Sender`.
pub struct Mailbox<M> {
    tx: mpsc::Sender<M>,
}

impl<M> Clone for Mailbox<M> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
}

impl<M: Send> Mailbox<M> {
    pub(crate) fn from_sender(tx: mpsc::Sender<M>) -> Self {
        Self { tx }
    }

    /// 메일박스가 가득 차면 여유가 생길 때까지 대기 (백프레셔).
    pub async fn send(&self, msg: M) -> Result<(), SendError> {
        self.tx.send(msg).await.map_err(|_| SendError::Closed)
    }

    /// 비대기 전송. 가득 차면 `Full` 반환 (느린 소비자 즉시 감지 → 끊기 정책에 사용).
    pub fn try_send(&self, msg: M) -> Result<(), SendError> {
        use mpsc::error::TrySendError;
        self.tx.try_send(msg).map_err(|e| match e {
            TrySendError::Full(_) => SendError::Full,
            TrySendError::Closed(_) => SendError::Closed,
        })
    }

    /// 현재 남은 용량.
    pub fn capacity(&self) -> usize {
        self.tx.capacity()
    }

    /// 액터가 살아있는지(수신측 존재).
    pub fn is_active(&self) -> bool {
        !self.tx.is_closed()
    }
}
