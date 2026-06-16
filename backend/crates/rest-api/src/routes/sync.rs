//! CRDT 오프라인 동기화 라우트 (개념: sync). D49, `user_crdt_entries`.
//!
//! 유저 동기화 문서(키별 LWW-Map). 여러 기기가 오프라인 편집 후 push해도 충돌 없이 수렴.
//! - `GET /users/@me/sync` — 현재 병합된 문서(살아있는 항목 + 원시 엔트리).
//! - `POST /users/@me/sync` — 기기의 로컬 상태(엔트리들)를 LWW 병합 → 병합된 문서 회신.
//!
//! 병합 권위는 domain `LwwMap`(순수), 영속은 storage의 LWW 가드 upsert. 상태 기반 동기화(CvRDT).

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use domain::crdt::{CrdtEntry, LwwMap};
use domain::repo::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new().route("/users/@me/sync", get(get_doc::<S>).post(push_doc::<S>))
}

/// 클라가 보내는 엔트리 1건. value 생략/null = 툼스톤(삭제). ts는 ms, node=복제본/기기 id.
#[derive(Deserialize)]
pub struct EntryIn {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
    pub ts: u64,
    pub node: u64,
}

#[derive(Deserialize)]
pub struct PushBody {
    #[serde(default)]
    pub entries: Vec<EntryIn>,
}

/// 응답: 살아있는 키→값 맵 + 원시 엔트리(툼스톤 포함, 클라가 자기 복제본과 다시 병합용).
#[derive(Serialize)]
pub struct DocView {
    pub live: Value,
    pub entries: Vec<EntryView>,
}

#[derive(Serialize)]
pub struct EntryView {
    pub key: String,
    pub value: Option<String>,
    pub ts: u64,
    pub node: u64,
}

fn view(doc: &LwwMap) -> DocView {
    let live: serde_json::Map<String, Value> =
        doc.live().into_iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect();
    let entries = doc
        .to_entries()
        .into_iter()
        .map(|e| EntryView { key: e.key, value: e.value, ts: e.ts_ms, node: e.node })
        .collect();
    DocView { live: Value::Object(live), entries }
}

async fn get_doc<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
) -> Result<Json<DocView>, ApiError> {
    let doc = st.store.load_user_doc(user).await?;
    Ok(Json(view(&doc)))
}

async fn push_doc<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Json(body): Json<PushBody>,
) -> Result<Json<DocView>, ApiError> {
    let entries: Vec<CrdtEntry> = body
        .entries
        .into_iter()
        .map(|e| CrdtEntry { key: e.key, value: e.value, ts_ms: e.ts, node: e.node })
        .collect();
    let merged = st.store.merge_user_doc(user, &entries).await?;
    Ok(Json(view(&merged)))
}
