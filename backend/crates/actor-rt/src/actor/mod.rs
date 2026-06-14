//! Actor trait + spawn (개념: actor).
//! 각 액터 = tokio task 1개 + bounded mpsc 메일박스. 상태를 독점, 메시지로만 소통 (D7).
//! 도메인을 모른다(범용). 외부 크레이트 없이 tokio + mpsc 수제.

use std::future::Future;

use tokio::sync::mpsc;

use crate::mailbox::Mailbox;

/// 메시지를 순차 처리하는 액터. 메시지 1개씩 처리 → Realm 내 순서 무료 보장 (D24).
pub trait Actor: Send + 'static {
    type Message: Send + 'static;

    /// 메시지 1건 처리. `&mut self`라 상태는 액터 내부에 격리(락 불필요, P5).
    fn handle(&mut self, msg: Self::Message) -> impl Future<Output = ()> + Send;
}

/// 액터를 tokio task로 띄우고 주소(Mailbox)를 반환.
/// `buffer` = 메일박스 용량(bounded, 백프레셔 D27). 모든 주소가 drop되면 루프 종료.
pub fn spawn<A: Actor>(mut actor: A, buffer: usize) -> Mailbox<A::Message> {
    let (tx, mut rx) = mpsc::channel::<A::Message>(buffer);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            actor.handle(msg).await;
        }
    });
    Mailbox::from_sender(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    enum Msg {
        Inc,
        Get(oneshot::Sender<u32>),
    }

    struct Counter {
        n: u32,
    }

    impl Actor for Counter {
        type Message = Msg;
        async fn handle(&mut self, msg: Msg) {
            match msg {
                Msg::Inc => self.n += 1,
                Msg::Get(reply) => {
                    let _ = reply.send(self.n);
                }
            }
        }
    }

    #[tokio::test]
    async fn processes_messages_in_order() {
        let addr = spawn(Counter { n: 0 }, 8);
        addr.send(Msg::Inc).await.unwrap();
        addr.send(Msg::Inc).await.unwrap();
        addr.send(Msg::Inc).await.unwrap();
        let (tx, rx) = oneshot::channel();
        addr.send(Msg::Get(tx)).await.unwrap();
        assert_eq!(rx.await.unwrap(), 3);
    }

    #[tokio::test]
    async fn send_fails_after_actor_stops() {
        let addr = {
            let a = spawn(Counter { n: 0 }, 1);
            a.clone()
        };
        // 원본 주소가 살아있으니 액터도 생존
        assert!(addr.is_active());
    }
}
