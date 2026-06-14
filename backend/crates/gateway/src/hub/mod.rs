//! 세션 레지스트리 (개념: hub). 이 노드의 로컬 세션 추적 + 팬아웃 배달.
//!
//! 세션 소유(D9): 클라가 붙은 노드가 그 세션을 보유. 한 유저가 여러 연결을 가질 수 있어
//! `user_id → [세션]`. LocalDelivery의 user_ids로 대상 세션을 찾아 ServerEvent를 push.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::protocol::ServerEvent;

#[derive(Clone)]
struct SessionHandle {
    session_id: u64,
    tx: mpsc::Sender<ServerEvent>,
}

/// 노드 로컬 세션 레지스트리. 복제해도 동일 내부를 공유(Arc).
#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Mutex<HashMap<u64, Vec<SessionHandle>>>>,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 세션 등록 → 이 세션으로의 이벤트 수신 채널을 반환.
    pub fn register(&self, user_id: u64, session_id: u64, buffer: usize) -> mpsc::Receiver<ServerEvent> {
        let (tx, rx) = mpsc::channel(buffer);
        self.inner
            .lock()
            .unwrap()
            .entry(user_id)
            .or_default()
            .push(SessionHandle { session_id, tx });
        rx
    }

    /// 세션 해제 (연결 종료 시).
    pub fn unregister(&self, user_id: u64, session_id: u64) {
        let mut map = self.inner.lock().unwrap();
        if let Some(v) = map.get_mut(&user_id) {
            v.retain(|h| h.session_id != session_id);
            if v.is_empty() {
                map.remove(&user_id);
            }
        }
    }

    /// 대상 유저들의 모든 로컬 세션에 이벤트 배달. 느린 세션(가득 참)은 건너뜀(백프레셔 D27).
    /// 락은 sender 복제 후 해제 → await은 락 밖에서.
    pub async fn deliver(&self, user_ids: &[u64], event: &ServerEvent) {
        let targets: Vec<mpsc::Sender<ServerEvent>> = {
            let map = self.inner.lock().unwrap();
            user_ids
                .iter()
                .filter_map(|u| map.get(u))
                .flat_map(|v| v.iter().map(|h| h.tx.clone()))
                .collect()
        };
        for tx in targets {
            // try_send: 느린 클라가 전체 팬아웃을 막지 않도록 (끊김은 후속 RESUME 정책, D24/D27).
            let _ = tx.try_send(event.clone());
        }
    }
}
