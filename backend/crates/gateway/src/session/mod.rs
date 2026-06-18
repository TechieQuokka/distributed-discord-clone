//! WS 세션 핸들러 (개념: session). 연결 수명주기 (gateway.md §2).
//!
//! HELLO → {IDENTIFY(신규) | RESUME(재개)} → ... → HEARTBEAT/DISPATCH 루프.
//! per-session seq + 재생 버퍼는 **Hub**(세션 소유 노드)가 보유(D24). 이 핸들러는 소켓 I/O만:
//! - IDENTIFY: 인증 → 세션 attach → READY(resume_token 포함) → 자동구독(D13) → 루프.
//! - RESUME: 토큰·seq 검증(Hub) → 놓친 프레임 재생 + RESUMED → 루프. 실패 시 INVALID_SESSION.

use axum::extract::ws::{Message, WebSocket};
use domain::id::{Snowflake, UserId};
use domain::repo::Store;
use node::Status;
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
#[allow(clippy::too_many_arguments)]
fn ready_payload(
    session_id: u64,
    resume_token: &str,
    user_id: u64,
    username: Option<&str>,
    realms: &[u64],
    read_states: Vec<Value>,
    presences: Vec<Value>,
    last_message_ids: Vec<Value>,
) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "resume_token": resume_token,
        "user": { "id": user_id.to_string(), "username": username },
        "realms": realms.iter().map(|r| json!({ "id": r.to_string() })).collect::<Vec<_>>(),
        "read_states": read_states,
        "presences": presences,
        // 채널별 마지막 메시지 id (D35/D48 warmup) — 클라 "최신으로 점프"용. 이벤트 로그 프로젝션으로
        // 콜드 Realm 액터를 복원하고 액터 권위값을 싣는다(원격 소유 realm은 프로젝션 값 직접).
        "last_message_ids": last_message_ids,
    })
}

/// 채널별 last_message_id 산출 + 로컬 액터 warmup (D35/D48). 이벤트 로그를 `RealmProjection`으로 재생해
/// 콜드 액터(failover/Q7 respawn)를 복원하고, 로컬 소유면 액터 권위값(warm+라이브 send 반영)을, 원격
/// 소유면 프로젝션 값(이 노드에 액터 없음)을 돌려준다. node는 IO 무지(P2)라 storage 읽기는 여기(엣지)서.
/// seam: realm마다 전체 재생(O(events)) — 스냅샷/컴팩션은 D48 후속.
async fn channel_last_ids<S: Store, T: NodeTransport>(
    store: &S,
    router: &node::Router<T>,
    realm: domain::id::RealmId,
) -> Vec<(u64, u64)> {
    let log = store.replay_events(realm, 0).await.unwrap_or_default();
    let proj = domain::event::RealmProjection::replay(&log);
    let warm: Vec<(u64, u64)> = proj.last_message_by_channel.iter().map(|(&c, &m)| (c, m)).collect();
    match router.warm_realm_last_ids(realm, warm.clone()).await {
        Some(live) => live, // 로컬 소유: 액터 권위값.
        None => warm,       // 원격 소유: 프로젝션 값 직접(seam: 원격 액터 warmup 조회 없음).
    }
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
    // 채널별 last_message_id (D35/D48 warmup) — 콜드 Realm 액터를 이벤트 로그 프로젝션으로 복원하며 산출.
    let mut last_message_ids: Vec<Value> = Vec::new();
    for r in &realm_ids {
        for (ch, mid) in channel_last_ids(&*state.store, &state.router, rid(*r)).await {
            last_message_ids.push(json!({
                "channel_id": ch.to_string(),
                "last_message_id": mid.to_string(),
            }));
        }
    }
    state.hub.dispatch_one(
        session_id,
        "READY",
        ready_payload(session_id, &resume_token, user_id, username.as_deref(), &realm_ids, read_states, presences, last_message_ids),
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

    pump(&mut socket, &state, Some(user_id), session_id, rx).await;
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
            pump(&mut socket, &state, user_id, session_id, rx).await;
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
            // 로컬에 세션 없음 → **크로스노드 RESUME**(세션 마이그레이션, D24): 원조 노드에서 핸드오프 시도.
            // 토큰 불일치/버퍼 gap이면 원조가 거부 → INVALID. 원조가 죽었으면(버퍼 휘발) 타임아웃 → INVALID.
            if let Some(m) = try_cross_node_resume(&state, session_id, &token, last_seq).await {
                let user_id = m.user_id;
                let rx = state.hub.import_migration(
                    session_id, user_id, m.last_seq, m.resume_token, DEFAULT_REPLAY_CAP,
                );
                // 원조 버퍼에서 받은 미수신 프레임 재생(원래 seq 보존) → RESUMED.
                let mut ok = true;
                for f in m.frames {
                    if socket.send(Message::Text(f.into())).await.is_err() {
                        ok = false;
                        break;
                    }
                }
                if ok && send(&mut socket, Outgoing::resumed(m.last_seq)).await.is_err() {
                    ok = false;
                }
                if !ok {
                    state.hub.detach(session_id);
                    return;
                }
                // 팬아웃 대상 활성화 + 유저 realm 재구독 → 구독자표(D12)를 이 노드로 이전(이벤트가 B로 흐름).
                state.hub.activate(user_id, session_id);
                let uid = UserId(Snowflake::from_raw(user_id));
                let realm_ids: Vec<u64> = state
                    .store
                    .member_realm_ids(uid)
                    .await
                    .unwrap_or_default()
                    .iter()
                    .map(|r| r.0.raw())
                    .collect();
                for r in &realm_ids {
                    let _ = state.router.route_subscribe(rid(*r), uid, state.local_node_id).await;
                }
                if state.hub.live_count(user_id) == 1 {
                    crate::presence::set_online(
                        &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, user_id,
                    )
                    .await;
                }
                pump(&mut socket, &state, Some(user_id), session_id, rx).await;
                state.hub.detach(session_id);
                if state.hub.live_count(user_id) == 0 {
                    crate::presence::set_offline(
                        &state.presence, &state.hub, &*state.store, &state.router, state.local_node_id, user_id,
                    )
                    .await;
                }
            } else {
                // 어느 노드에도 없음/거부 → 재IDENTIFY + REST 재조회 유도 (D24).
                let _ = send(&mut socket, Outgoing::invalid_session()).await;
            }
        }
    }
}

/// 크로스노드 RESUME (D24): 원조 노드에 `ResumeFetch` broadcast → `ResumeState` 대기(타임아웃).
/// 원조만 세션을 가져 응답한다(server inbound가 `export_migration`→회신, `complete_migration`로 깨움).
async fn try_cross_node_resume<S: Store, T: NodeTransport>(
    state: &GatewayState<S, T>,
    session_id: u64,
    token: &str,
    last_seq: u64,
) -> Option<crate::hub::MigratedSession> {
    let rx = state.hub.begin_migration(session_id);
    state
        .router
        .broadcast(protocol::NodeMessage::ResumeFetch {
            session_id,
            token: token.to_string(),
            last_seq,
            requester: state.local_node_id,
        })
        .await;
    match tokio::time::timeout(std::time::Duration::from_millis(2000), rx).await {
        Ok(Ok(m)) if m.found => Some(m),
        _ => {
            state.hub.cancel_migration(session_id);
            None
        }
    }
}

/// 메인 루프: 클라 수신(하트비트/상태변경 등) ↔ Hub 배달 프레임 송신. 끊김/종료 시 반환.
async fn pump<S: Store + 'static, T: NodeTransport>(
    socket: &mut WebSocket,
    state: &GatewayState<S, T>,
    user_id: Option<u64>,
    session_id: u64,
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
                                // 클라 상태 변경(online/idle/dnd, D42). offline은 무시(연결 중엔 online 계열만).
                                op::PRESENCE_UPDATE => {
                                    if let (Some(u), Some(st)) = (user_id, parse_presence_status(&inc.d)) {
                                        crate::presence::set_status(
                                            &state.presence, &state.hub, &*state.store, &state.router,
                                            state.local_node_id, u, st,
                                        )
                                        .await;
                                    }
                                }
                                // 음성 시그널링(D47): 입장/이동/퇴장 + self mute/deaf → 권한(CONNECT) →
                                // 같은 Realm 구독자에 VOICE_STATE_UPDATE 팬아웃(기존 emit 경로 재사용, 신규 와이어 0).
                                op::VOICE_STATE_UPDATE => {
                                    if let Some(u) = user_id {
                                        handle_voice_state(state, session_id, u, &inc.d).await;
                                    }
                                }
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

/// 음성 시그널링 op4 처리 (D47, 미디어 제외 D21). 입장/이동/퇴장 + self mute/deaf를 같은 Realm
/// 구독자에 `VOICE_STATE_UPDATE`로 팬아웃(기존 emit 경로 재사용 — 신규 와이어/액터 상태 0, 스펙 §3).
/// 입장 시 `VOICE_SERVER_UPDATE`(endpoint=null stub)를 이 세션에만 회신해 미디어 경계를 표식.
async fn handle_voice_state<S: Store, T: NodeTransport>(
    state: &GatewayState<S, T>,
    session_id: u64,
    user_id: u64,
    d: &Value,
) {
    let parse_id = |v: &Value| v.as_str().and_then(|s| s.parse::<u64>().ok());
    let Some(realm_raw) = d.get("realm_id").and_then(parse_id) else { return };
    let realm = rid(realm_raw);
    // channel_id null/누락 = 채널 떠남(퇴장).
    let channel_raw: Option<u64> = d.get("channel_id").and_then(parse_id);
    let self_mute = d.get("self_mute").and_then(Value::as_bool).unwrap_or(false);
    let self_deaf = d.get("self_deaf").and_then(Value::as_bool).unwrap_or(false);

    // 입장/이동: CONNECT 권한(채널 컨텍스트, D17/D47). 퇴장(None)은 무권한. 없으면 무시(Discord식).
    if let Some(ch) = channel_raw {
        use domain::permissions::Permissions;
        let granted = crate::routes::effective_channel_perms(
            &*state.store,
            domain::id::ChannelId(Snowflake::from_raw(ch)),
            realm,
            UserId(Snowflake::from_raw(user_id)),
        )
        .await
        .ok()
        .flatten()
        .is_some_and(|p| p.contains(Permissions::CONNECT));
        if !granted {
            return;
        }
    }

    // VOICE_STATE_UPDATE payload (생산 엣지가 JSON 단일 출처, D39). server_mute/deaf 모더레이션은 seam→false.
    let payload = json!({
        "realm_id": realm_raw.to_string(),
        "channel_id": channel_raw.map(|c| c.to_string()),
        "user_id": user_id.to_string(),
        "self_mute": self_mute,
        "self_deaf": self_deaf,
        "server_mute": false,
        "server_deaf": false,
    })
    .to_string();
    // 같은 Realm 구독자에 팬아웃 — 멤버 이벤트와 같은 emit 경로(로컬/원격 자동, 이벤트소싱 fact 없음).
    let _ = state.router.route_emit(realm, "VOICE_STATE_UPDATE".into(), payload, None).await;

    // 입장 확정 시 VOICE_SERVER_UPDATE를 이 세션에만 회신 — 미디어 서버 경계(D21): endpoint=null stub.
    if channel_raw.is_some() {
        state.hub.dispatch_one(
            session_id,
            "VOICE_SERVER_UPDATE",
            json!({
                "realm_id": realm_raw.to_string(),
                "endpoint": Value::Null,         // 미디어 SFU 없음(D21) — 시그널링 종료 표식.
                "token": session_id.to_string(), // 미디어 인증 토큰은 설계만(stub).
            }),
        );
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

/// op 3 페이로드 `{ "status": "online"|"idle"|"dnd" }`를 파싱. 연결 중엔 online 계열만 허용
/// (offline/미상은 None → 무시; 오프라인 전이는 세션 종료가 담당).
fn parse_presence_status(d: &Value) -> Option<Status> {
    match d.get("status").and_then(|s| s.as_str()) {
        Some("online") => Some(Status::Online),
        Some("idle") => Some(Status::Idle),
        Some("dnd") => Some(Status::Dnd),
        _ => None,
    }
}

async fn send(socket: &mut WebSocket, out: Outgoing) -> Result<(), axum::Error> {
    socket.send(Message::Text(out.to_json().into())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_presence_status_accepts_online_family_only() {
        assert_eq!(parse_presence_status(&json!({ "status": "online" })), Some(Status::Online));
        assert_eq!(parse_presence_status(&json!({ "status": "idle" })), Some(Status::Idle));
        assert_eq!(parse_presence_status(&json!({ "status": "dnd" })), Some(Status::Dnd));
        // offline/미상/누락은 None → op 3 무시 (오프라인 전이는 세션 종료가 담당).
        assert_eq!(parse_presence_status(&json!({ "status": "offline" })), None);
        assert_eq!(parse_presence_status(&json!({ "status": "bogus" })), None);
        assert_eq!(parse_presence_status(&json!({})), None);
    }
}
