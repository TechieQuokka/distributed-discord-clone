//! WS 세션 핸들러 (개념: session). 연결 수명주기 (gateway.md §2).
//!
//! HELLO → IDENTIFY(access 토큰 검증) → READY(자동구독 D13) → HEARTBEAT/DISPATCH 루프.
//! per-session 시퀀스 `s`는 이 노드(세션 소유)가 부여(D24). RESUME 재생버퍼는 Phase 2.

use axum::extract::ws::{Message, WebSocket};
use domain::id::{Snowflake, UserId};
use domain::repo::Store;
use serde_json::{Value, json};
use transport::NodeTransport;

use crate::protocol::{IdentifyData, Incoming, Outgoing, op};
use crate::state::GatewayState;

/// READY 페이로드(초기 스냅샷). 최소 버전: session_id, user, realm id 목록.
fn ready_payload(session_id: u64, user_id: u64, username: Option<&str>, realms: &[u64]) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "user": { "id": user_id.to_string(), "username": username },
        "realms": realms.iter().map(|r| json!({ "id": r.to_string() })).collect::<Vec<_>>(),
    })
}

pub async fn handle_socket<S: Store + 'static, T: NodeTransport>(
    mut socket: WebSocket,
    state: GatewayState<S, T>,
) {
    // HELLO (heartbeat 권고).
    if send(&mut socket, Outgoing::hello(state.heartbeat_interval_ms)).await.is_err() {
        return;
    }

    // IDENTIFY 대기 (성공 시 user_id).
    let user_id = match await_identify(&mut socket, &state).await {
        Some(uid) => uid,
        None => return, // 인증 실패/끊김 → 핸들러 종료.
    };

    let session_id = state.snowflakes.next(state.clock.now_ms()).raw();
    let mut rx = state.hub.register(user_id, session_id, 256);
    let uid = UserId(Snowflake::from_raw(user_id));

    // 자동 구독 (D13): 멤버인 Realm들의 이벤트를 받도록 등록.
    let realm_ids: Vec<u64> = state
        .store
        .member_realm_ids(uid)
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.0.raw())
        .collect();
    for r in &realm_ids {
        let _ = state
            .router
            .route_subscribe(rid(*r), uid, state.local_node_id)
            .await;
    }

    // READY.
    let username = state.store.find_by_id(uid).await.ok().flatten().map(|u| u.username);
    let mut seq = 0u64;
    seq += 1;
    let ready = Outgoing::dispatch(seq, "READY", ready_payload(session_id, user_id, username.as_deref(), &realm_ids));
    if send(&mut socket, ready).await.is_err() {
        state.hub.unregister(user_id, session_id);
        return;
    }

    // 메인 루프: 클라 메시지 수신 ↔ 디스패치 이벤트 송신.
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(inc) = serde_json::from_str::<Incoming>(txt.as_str()) {
                            match inc.op {
                                op::HEARTBEAT => {
                                    if send(&mut socket, Outgoing::heartbeat_ack()).await.is_err() { break; }
                                }
                                // RESUME 재생버퍼는 Phase 2 — 지금은 재IDENTIFY 유도.
                                op::RESUME => { let _ = send(&mut socket, Outgoing::invalid_session()).await; }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}        // ping/pong/binary 무시.
                    Some(Err(_)) => break,
                }
            }
            event = rx.recv() => {
                match event {
                    Some(ev) => {
                        seq += 1;
                        if send(&mut socket, Outgoing::dispatch(seq, ev.t, ev.d)).await.is_err() { break; }
                    }
                    None => break,            // hub drop (서버 종료).
                }
            }
        }
    }

    state.hub.unregister(user_id, session_id);
}

/// IDENTIFY를 받아 토큰 검증 → user_id. 실패 시 INVALID_SESSION 후 None.
async fn await_identify<S: Store + 'static, T: NodeTransport>(
    socket: &mut WebSocket,
    state: &GatewayState<S, T>,
) -> Option<u64> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(txt))) => {
                let Ok(inc) = serde_json::from_str::<Incoming>(txt.as_str()) else { continue };
                match inc.op {
                    op::IDENTIFY => {
                        let Ok(data) = serde_json::from_value::<IdentifyData>(inc.d) else {
                            let _ = send(socket, Outgoing::invalid_session()).await;
                            return None;
                        };
                        match state.keys.verify_access(&data.token) {
                            Ok(uid) => return Some(uid),
                            Err(_) => {
                                let _ = send(socket, Outgoing::invalid_session()).await;
                                return None;
                            }
                        }
                    }
                    op::HEARTBEAT => {
                        if send(socket, Outgoing::heartbeat_ack()).await.is_err() {
                            return None;
                        }
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Close(_))) | None => return None,
            Some(Ok(_)) => {}
            Some(Err(_)) => return None,
        }
    }
}

fn rid(raw: u64) -> domain::id::RealmId {
    domain::id::RealmId(Snowflake::from_raw(raw))
}

async fn send(socket: &mut WebSocket, out: Outgoing) -> Result<(), axum::Error> {
    socket.send(Message::Text(out.to_json().into())).await
}
