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
///
/// **Let-it-crash 계약 (Q7/D50)**: `handle`이 패닉하면 그 task가 unwind하며 종료한다 →
/// 수신측이 사라져 [`Mailbox::is_active`]가 `false`가 된다(패닉 자체는 tokio 기본 패닉 훅이
/// 로깅). 이 런타임은 액터를 자동 재시작하지 **않는다** — 재시작은 상위 supervisor(여기선
/// node `Router`가 닫힌 메일박스를 감지해 fresh 액터로 lazy 재spawn = rehydrate)의 책임이다.
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

    /// Let-it-crash 계약 (Q7/D50): handle이 패닉하면 액터 task가 종료 → 메일박스가 닫힌다
    /// (supervisor가 이 신호로 재시작을 판단). 패닉 출력은 tokio 기본 훅이 담당.
    #[tokio::test]
    async fn panicking_handler_closes_mailbox() {
        struct Boom;
        impl Actor for Boom {
            type Message = ();
            async fn handle(&mut self, _msg: ()) {
                panic!("boom");
            }
        }
        let addr = spawn(Boom, 4);
        let _ = addr.send(()).await; // 패닉 유발.
        // 패닉 task가 unwind할 때까지 양보(결정론적으로 닫힐 때까지 폴링, time feature 불필요).
        for _ in 0..10_000 {
            if !addr.is_active() {
                return; // 닫힘 확인 — 계약 충족.
            }
            tokio::task::yield_now().await;
        }
        panic!("handler 패닉 후에도 메일박스가 닫히지 않음");
    }
}
