//! WS 세션 핸들러 (개념: session). 연결 수명주기 (gateway.md §2).
//!
//! HELLO → {IDENTIFY(신규) | RESUME(재개)} → ... → HEARTBEAT/DISPATCH 루프.
//! per-session seq + 재생 버퍼는 **Hub**(세션 소유 노드)가 보유(D24). 이 핸들러는 소켓 I/O만:
//! - IDENTIFY: 인증 → 세션 attach → READY(resume_token 포함) → 자동구독(D13) → 루프.
//! - RESUME: 토큰·seq 검증(Hub) → 놓친 프레임 재생 + RESUMED → 루프. 실패 시 INVALID_SESSION.

use axum::extract::ws::{Message, WebSocket};
use domain::id::{Snowflake, UserId};
use domain::repo::Store;
use serde_json::{Value, json};
use transport::NodeTransport;

use crate::hub::{DEFAULT_REPLAY_CAP, ResumeOutcome};
use crate::protocol::{IdentifyData, Incoming, Outgoing, ResumeData, op};
use crate::state::GatewayState;

/// 클라 핸드셰이크 결과.
enum Handshake {
    Identify { user_id: u64 },
    Resume { session_id: u64, token: String, seq: u64 },
}

/// READY 페이로드(초기 스냅샷). session_id + resume_token(D20) + user + realm id 목록 + 읽음 상태 + 친구 presence.
fn ready_payload(
    session_id: u64,
    resume_token: &str,
    user_id: u64,
    username: Option<&str>,
    realms: &[u64],
    read_states: Vec<Value>,
    presences: Vec<Value>,
) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "resume_token": resume_token,
        "user": { "id": user_id.to_string(), "username": username },
        "realms": realms.iter().map(|r| json!({ "id": r.to_string() })).collect::<Vec<_>>(),
        "read_states": read_states,
        "presences": presences,
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

    // 핸드셰이크: IDENTIFY(신규) 또는 RESUME(재개) 대기.
    match await_handshake(&mut socket, &state).await {
        Some(Handshake::Identify { user_id }) => run_identify(socket, state, user_id).await,
        Some(Handshake::Resume { session_id, token, seq }) => {
            run_resume(socket, state, session_id, token, seq).await
        }
        None => {} // 인증 실패/끊김 → 핸들러 종료.
    }
}

/// 신규 세션: attach → READY → 자동구독 → 루프.
async fn run_identify<S: Store + 'static, T: NodeTransport>(
    mut socket: WebSocket,
    state: GatewayState<S, T>,
    user_id: u64,
) {
    let session_id = state.snowflakes.next(state.clock.now_ms()).raw();
    // attach: 세션 등록(아직 팬아웃 비대상). READY가 seq=1로 먼저 가도록 activate는 뒤에.
    let (rx, resume_token) = state.hub.attach(user_id, session_id, DEFAULT_REPLAY_CAP);
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

    // READY (seq=1) — activate 이전이라 팬아웃과 경합 없음.
    let username = state.store.find_by_id(uid).await.ok().flatten().map(|u| u.username);
    let read_states: Vec<Value> = state
        .store
        .list_read_states(uid)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            json!({
                "channel_id": s.channel_id.0.raw().to_string(),
                "last_read_message_id": s.last_read_message_id.map(|m| m.0.raw().to_string()),
                "mention_count": s.mention_count,
            })
        })
        .collect();
    // 친구 presence 스냅샷(현재 온라인 친구).
    let presences = crate::presence::ready_presences(&*state.store, &state.presence, user_id).await;
    state.hub.dispatch_one(
        session_id,
        "READY",
        ready_payload(session_id, &resume_token, user_id, username.as_deref(), &realm_ids, read_states, presences),
    );

    // 이제 팬아웃 대상으로 활성화 + Realm 구독.
    state.hub.activate(user_id, session_id);
    for r in &realm_ids {
        let _ = state.router.route_subscribe(rid(*r), uid, state.local_node_id).await;
    }
    // presence 온라인 전이 (이 유저의 첫 live 세션일 때만 gossip+통지).
    if state.hub.live_count(user_id) == 1 {
        crate::presence::set_online(
            &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, user_id,
        )
        .await;
    }

    pump(&mut socket, &state, rx).await;
    state.hub.detach(session_id); // 끊김: 버퍼 유지(RESUME 대비). grace 후 Hub가 purge.
    // 마지막 live 세션이 끊겼으면 presence 오프라인 전이.
    if state.hub.live_count(user_id) == 0 {
        crate::presence::set_offline(
            &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, user_id,
        )
        .await;
    }
}

/// 재개: Hub에서 토큰·seq 검증 → 놓친 프레임 재생 + RESUMED → 루프.
async fn run_resume<S: Store + 'static, T: NodeTransport>(
    mut socket: WebSocket,
    state: GatewayState<S, T>,
    session_id: u64,
    token: String,
    last_seq: u64,
) {
    match state.hub.resume(session_id, &token, last_seq, DEFAULT_REPLAY_CAP) {
        ResumeOutcome::Resumed { rx, replay, last_seq } => {
            let user_id = state.hub.session_user(session_id);
            // 놓친 프레임 직접 재생(원래 seq 보존) → RESUMED.
            for frame in replay {
                if send(&mut socket, frame).await.is_err() {
                    state.hub.detach(session_id);
                    return;
                }
            }
            if send(&mut socket, Outgoing::resumed(last_seq)).await.is_err() {
                state.hub.detach(session_id);
                return;
            }
            // 재개로 다시 live → 첫 live 세션이면 presence 온라인 전이(grace 중 offline됐던 경우 복귀).
            if let Some(u) = user_id
                && state.hub.live_count(u) == 1
            {
                crate::presence::set_online(
                    &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, u,
                )
                .await;
            }
            pump(&mut socket, &state, rx).await;
            state.hub.detach(session_id);
            if let Some(u) = user_id
                && state.hub.live_count(u) == 0
            {
                crate::presence::set_offline(
                    &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, u,
                )
                .await;
            }
        }
        ResumeOutcome::Invalid => {
            // 버퍼 밖/토큰 불일치 → 재IDENTIFY + REST 재조회 유도 (D24).
            let _ = send(&mut socket, Outgoing::invalid_session()).await;
        }
    }
}

/// 메인 루프: 클라 수신(하트비트 등) ↔ Hub 배달 프레임 송신. 끊김/종료 시 반환.
async fn pump<S: Store + 'static, T: NodeTransport>(
    socket: &mut WebSocket,
    _state: &GatewayState<S, T>,
    mut rx: tokio::sync::mpsc::Receiver<Outgoing>,
) {
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(inc) = serde_json::from_str::<Incoming>(txt.as_str()) {
                            match inc.op {
                                op::HEARTBEAT
                                    if send(socket, Outgoing::heartbeat_ack()).await.is_err() => { break; }
                                _ => {} // RESUME/IDENTIFY는 핸드셰이크 단계 전용. 그 외 무시.
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}        // ping/pong/binary 무시.
                    Some(Err(_)) => break,
                }
            }
            frame = rx.recv() => {
                match frame {
                    // Hub가 seq를 이미 부여한 완성 프레임 — 그대로 전달.
                    Some(out) => { if send(socket, out).await.is_err() { break; } }
                    None => break, // live 교체(RESUME 슈퍼시드)/Hub drop → 이 소켓 종료.
                }
            }
        }
    }
}

/// 핸드셰이크 수신: IDENTIFY(토큰 검증→user_id) 또는 RESUME(세션 재개 요청). 실패 시 None.
async fn await_handshake<S: Store + 'static, T: NodeTransport>(
    socket: &mut WebSocket,
    state: &GatewayState<S, T>,
) -> Option<Handshake> {
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
                            Ok(uid) => return Some(Handshake::Identify { user_id: uid }),
                            Err(_) => {
                                let _ = send(socket, Outgoing::invalid_session()).await;
                                return None;
                            }
                        }
                    }
                    op::RESUME => {
                        let Ok(data) = serde_json::from_value::<ResumeData>(inc.d) else {
                            let _ = send(socket, Outgoing::invalid_session()).await;
                            return None;
                        };
                        match data.session_id.parse::<u64>() {
                            Ok(sid) => {
                                return Some(Handshake::Resume {
                                    session_id: sid,
                                    token: data.token,
                                    seq: data.seq,
                                });
                            }
                            Err(_) => {
                                let _ = send(socket, Outgoing::invalid_session()).await;
                                return None;
                            }
                        }
                    }
                    op::HEARTBEAT
                        if send(socket, Outgoing::heartbeat_ack()).await.is_err() => {
                            return None;
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
