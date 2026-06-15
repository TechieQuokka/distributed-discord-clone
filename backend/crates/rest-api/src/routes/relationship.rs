//! 친구·차단 라우트 (개념: routes/relationship). rest.md §2, 스키마 §6, gateway RELATIONSHIP_*.
//!
//! - `GET /users/@me/relationships` — 내 관계 목록(친구/대기/차단).
//! - `PUT /users/@me/relationships/{user_id}` — `{ "type": "friend" }`(요청/수락) 또는 `"block"`(차단).
//! - `DELETE /users/@me/relationships/{user_id}` — 친구 삭제/요청 취소·거절/차단 해제.
//!
//! 실시간 통지는 **유저 단위** `UserEmitter`로 `RELATIONSHIP_ADD/_REMOVE`(Realm 무관). 크로스노드
//! 유저 라우팅은 전역 presence/gossip(Q11) seam — 현재는 대상이 이 노드에 접속 중이면 즉시 수신.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use domain::id::{Snowflake, UserId};
use domain::relationship::RelationKind;
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::{relationship_add_payload, relationship_remove_payload};
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/users/@me/relationships", get(list_relationships::<S>))
        .route(
            "/users/@me/relationships/{user_id}",
            axum::routing::put(put_relationship::<S>).delete(delete_relationship::<S>),
        )
}

fn parse_user(s: &str) -> Result<UserId, ApiError> {
    s.parse::<u64>()
        .map(|n| UserId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid user id".into()))
}

#[derive(Serialize)]
pub struct RelationshipView {
    pub user_id: String,
    pub kind: String,
}

async fn list_relationships<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(me): AuthUser,
) -> Result<Json<Vec<RelationshipView>>, ApiError> {
    let rels = st.store.list_relationships(me).await?;
    Ok(Json(
        rels.into_iter()
            .map(|r| RelationshipView { user_id: r.target_id.0.raw().to_string(), kind: r.kind.as_str().to_string() })
            .collect(),
    ))
}

#[derive(Deserialize, Default)]
pub struct PutRelationshipReq {
    /// "friend"(요청/수락) | "block"(차단). 생략 시 friend.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

async fn put_relationship<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(me): AuthUser,
    Path(user_id): Path<String>,
    body: Option<Json<PutRelationshipReq>>,
) -> Result<Json<RelationshipView>, ApiError> {
    let target = parse_user(&user_id)?;
    if target == me {
        return Err(ApiError::BadRequest("cannot relate to yourself".into()));
    }
    if st.store.find_by_id(target).await?.is_none() {
        return Err(ApiError::BadRequest("user does not exist".into()));
    }
    let req = body.map(|Json(b)| b).unwrap_or_default();

    match req.kind.as_deref().unwrap_or("friend") {
        "block" => {
            st.store.block(me, target).await?;
            // 상대는 관계가 사라짐 → REMOVE. 나는 차단 추가 → ADD(blocked).
            if let Some(t) = st.store.find_by_id(target).await? {
                let payload = relationship_add_payload(&t, RelationKind::Blocked);
                let _ = st.user_emitter.emit_to_users(&[me], "RELATIONSHIP_ADD".into(), payload).await;
            }
            let _ = st
                .user_emitter
                .emit_to_users(&[target], "RELATIONSHIP_REMOVE".into(), relationship_remove_payload(me))
                .await;
            Ok(Json(RelationshipView { user_id: target.0.raw().to_string(), kind: "blocked".into() }))
        }
        "friend" => {
            // 차단 관계면 친구 불가.
            if st.store.get_relationship(me, target).await? == Some(RelationKind::Blocked) {
                return Err(ApiError::BadRequest("unblock the user first".into()));
            }
            if st.store.get_relationship(target, me).await? == Some(RelationKind::Blocked) {
                return Err(ApiError::Forbidden); // 상대가 나를 차단.
            }
            let result = st.store.friend_request_or_accept(me, target).await?;

            // 각자 관점으로 RELATIONSHIP_ADD 통지(요청 방향이 뒤집힘).
            if let (Some(target_u), Some(me_u)) =
                (st.store.find_by_id(target).await?, st.store.find_by_id(me).await?)
            {
                let _ = st
                    .user_emitter
                    .emit_to_users(&[me], "RELATIONSHIP_ADD".into(), relationship_add_payload(&target_u, result))
                    .await;
                let _ = st
                    .user_emitter
                    .emit_to_users(
                        &[target],
                        "RELATIONSHIP_ADD".into(),
                        relationship_add_payload(&me_u, result.mirror()),
                    )
                    .await;
            }
            Ok(Json(RelationshipView { user_id: target.0.raw().to_string(), kind: result.as_str().to_string() }))
        }
        _ => Err(ApiError::BadRequest("type must be 'friend' or 'block'".into())),
    }
}

async fn delete_relationship<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(me): AuthUser,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let target = parse_user(&user_id)?;
    let removed = st.store.remove_relationship(me, target).await?.ok_or(ApiError::NotFound)?;

    // 나는 항상 REMOVE. 친구/대기였다면 상대도 REMOVE(차단 해제는 상대 영향 없음).
    let _ = st
        .user_emitter
        .emit_to_users(&[me], "RELATIONSHIP_REMOVE".into(), relationship_remove_payload(target))
        .await;
    if removed != RelationKind::Blocked {
        let _ = st
            .user_emitter
            .emit_to_users(&[target], "RELATIONSHIP_REMOVE".into(), relationship_remove_payload(me))
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
