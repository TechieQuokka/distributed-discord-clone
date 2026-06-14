//! Gateway JSON 와이어 (개념: protocol). docs/api/gateway.md §0-1.
//!
//! 공통 프레임 `{ op, d, s?, t? }`. 클라 엣지는 JSON(D31). 노드↔노드 수제 바이트와 별개.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Opcodes (gateway.md §1).
pub mod op {
    pub const DISPATCH: u8 = 0;
    pub const HEARTBEAT: u8 = 1;
    pub const IDENTIFY: u8 = 2;
    pub const RESUME: u8 = 6;
    pub const INVALID_SESSION: u8 = 9;
    pub const HELLO: u8 = 10;
    pub const HEARTBEAT_ACK: u8 = 11;
}

/// 클라 → 서버 수신 프레임. `op`로 분기, `d`는 op별 페이로드.
#[derive(Deserialize)]
pub struct Incoming {
    pub op: u8,
    #[serde(default)]
    pub d: Value,
}

/// IDENTIFY 페이로드.
#[derive(Deserialize)]
pub struct IdentifyData {
    pub token: String,
}

/// RESUME 페이로드 (gateway.md §2). 세션 재개 자격증명 + 마지막 수신 seq.
#[derive(Deserialize)]
pub struct ResumeData {
    /// 세션 id (READY에서 받은 문자열).
    pub session_id: String,
    /// CSPRNG resume 토큰 (READY에서 받음, D20).
    pub token: String,
    /// 클라가 마지막으로 받은 dispatch seq.
    #[serde(default)]
    pub seq: u64,
}

/// 서버 → 클라 송신 프레임. DISPATCH만 `s`/`t` 포함.
#[derive(Clone, Serialize)]
pub struct Outgoing {
    pub op: u8,
    pub d: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

impl Outgoing {
    pub fn control(op: u8, d: Value) -> Self {
        Self { op, d, s: None, t: None }
    }
    pub fn hello(heartbeat_interval_ms: u64) -> Self {
        Self::control(op::HELLO, serde_json::json!({ "heartbeat_interval": heartbeat_interval_ms }))
    }
    pub fn heartbeat_ack() -> Self {
        Self::control(op::HEARTBEAT_ACK, Value::Null)
    }
    pub fn invalid_session() -> Self {
        Self::control(op::INVALID_SESSION, Value::Bool(false))
    }
    /// DISPATCH: 이벤트 이름 `t` + per-session 시퀀스 `s` (D24).
    pub fn dispatch(seq: u64, t: impl Into<String>, d: Value) -> Self {
        Self { op: op::DISPATCH, d, s: Some(seq), t: Some(t.into()) }
    }
    /// RESUMED: 재생 완료 신호. DISPATCH 형태(t="RESUMED"), 현재 마지막 seq를 실어 보냄
    /// (새 이벤트 아님 → 버퍼/seq 증가 없음, gateway.md §2).
    pub fn resumed(last_seq: u64) -> Self {
        Self { op: op::DISPATCH, d: Value::Null, s: Some(last_seq), t: Some("RESUMED".into()) }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// 디스패치할 논리 이벤트 (seq 미포함 — 세션마다 자체 seq 부여, D24).
#[derive(Clone, Debug)]
pub struct ServerEvent {
    pub t: String,
    pub d: Value,
}
