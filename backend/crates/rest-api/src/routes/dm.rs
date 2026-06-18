//! DM/그룹DM 라우트 (개념: routes/dm). D8 Realm 통일, DB-D2, rest.md §2/§4.
//!
//! - `POST /users/@me/channels` — 1:1 DM(`recipient_id`) 또는 그룹DM(`recipient_ids`) 열기.
//!   1:1은 `dm_pairs`로 중복 방지(find-or-create) → 기존 있으면 그 채널 재사용.
//! - `PUT/DELETE /channels/:id/recipients/:uid` — 그룹DM 참가자 추가/제거(소유자) 또는 탈퇴(본인).
//!
//! DM Realm은 @everyone 역할이 없어 권한 계산이 `default_everyone`으로 폴백한다 → 멤버면
//! 전송·조회·리액션이 길드와 **같은 경로**로 동작(추가 분기 없음, P4). 차단(blocked) 거부는
//! relationships 도입 후의 seam(permissions.md §5).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{post, put};
use domain::dm::{NewDm, NewGroupDm, RealmKind};
use domain::id::{ChannelId, RealmId, Snowflake, UserId};
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::{recipient_add_payload, recipient_remove_payload};
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/users/@me/channels", post(open_channel::<S>))
        .route(
            "/channels/{channel_id}/recipients/{user_id}",
            put(add_recipient::<S>).delete(remove_recipient::<S>),
        )
}

fn parse_channel(s: &str) -> Result<ChannelId, ApiError> {
    s.parse::<u64>()
        .map(|n| ChannelId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid channel id".into()))
}
fn parse_user(s: &str) -> Result<UserId, ApiError> {
    s.parse::<u64>()
        .map(|n| UserId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid user id".into()))
}

#[derive(Deserialize)]
pub struct OpenChannelReq {
    /// 1:1 DM 상대 user id.
    #[serde(default)]
    pub recipient_id: Option<String>,
    /// 그룹DM 참가자 user id 목록 (호출자 제외).
    #[serde(default)]
    pub recipient_ids: Vec<String>,
    /// 그룹DM 이름 (선택).
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct DmChannelView {
    pub id: String,
    pub realm_id: String,
    pub kind: String,
    pub recipients: Vec<String>,
}

/// DM(1:1) 또는 그룹DM 열기. 1:1은 find-or-create(중복 방지), 그룹은 새로 생성.
async fn open_channel<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(me): AuthUser,
    Json(req): Json<OpenChannelReq>,
) -> Result<(StatusCode, Json<DmChannelView>), ApiError> {
    // 그룹DM: recipient_ids 가 채워졌으면 그룹 경로.
    if !req.recipient_ids.is_empty() {
        return open_group(&st, me, req).await;
    }
    // 1:1 DM.
    let other = req
        .recipient_id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("recipient_id or recipient_ids required".into()))?;
    let other = parse_user(other)?;
    if other == me {
        return Err(ApiError::BadRequest("cannot DM yourself".into()));
    }
    if st.store.find_by_id(other).await?.is_none() {
        return Err(ApiError::BadRequest("recipient does not exist".into()));
    }
    // 차단 관계면 1:1 DM 거부 (permissions.md §5).
    if st.store.is_blocked_between(me, other).await? {
        return Err(ApiError::Forbidden);
    }

    // 이미 있으면 재사용 (200).
    if let Some(existing) = st.store.find_dm(me, other).await? {
        return Ok((StatusCode::OK, Json(dm_view(existing.realm_id, existing.channel_id, RealmKind::Dm, vec![me, other]))));
    }

    // 생성. 레이스로 Conflict면 재조회 후 반환(멱등).
    let realm_id = RealmId(st.snowflakes.next(st.clock.now_ms()));
    let channel_id = ChannelId(st.snowflakes.next(st.clock.now_ms()));
    match st
        .store
        .create_dm(&NewDm { realm_id, channel_id, user_a: me, user_b: other })
        .await
    {
        Ok(()) => Ok((StatusCode::CREATED, Json(dm_view(realm_id, channel_id, RealmKind::Dm, vec![me, other])))),
        Err(domain::repo::RepoError::Conflict) => {
            let existing = st
                .store
                .find_dm(me, other)
                .await?
                .ok_or_else(|| ApiError::Conflict("dm create race".into()))?;
            Ok((StatusCode::OK, Json(dm_view(existing.realm_id, existing.channel_id, RealmKind::Dm, vec![me, other]))))
        }
        Err(e) => Err(e.into()),
    }
}

async fn open_group<S: Store + 'static>(
    st: &AppState<S>,
    me: UserId,
    req: OpenChannelReq,
) -> Result<(StatusCode, Json<DmChannelView>), ApiError> {
    // 참가자 파싱 + 중복/본인 제거 + 존재 검증.
    let mut members: Vec<UserId> = vec![me];
    for s in &req.recipient_ids {
        let u = parse_user(s)?;
        if u == me || members.contains(&u) {
            continue;
        }
        if st.store.find_by_id(u).await?.is_none() {
            return Err(ApiError::BadRequest("recipient does not exist".into()));
        }
        members.push(u);
    }
    if members.len() < 2 {
        return Err(ApiError::BadRequest("group DM needs at least one other recipient".into()));
    }

    let realm_id = RealmId(st.snowflakes.next(st.clock.now_ms()));
    let channel_id = ChannelId(st.snowflakes.next(st.clock.now_ms()));
    let name = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    st.store
        .create_group_dm(&NewGroupDm { realm_id, channel_id, owner: me, name, members: members.clone() })
        .await?;
    Ok((StatusCode::CREATED, Json(dm_view(realm_id, channel_id, RealmKind::GroupDm, members))))
}

fn dm_view(realm: RealmId, channel: ChannelId, kind: RealmKind, recipients: Vec<UserId>) -> DmChannelView {
    DmChannelView {
        id: channel.0.raw().to_string(),
        realm_id: realm.0.raw().to_string(),
        kind: kind.as_str().to_string(),
        recipients: recipients.iter().map(|u| u.0.raw().to_string()).collect(),
    }
}

/// 그룹DM의 realm 메타를 검증해 반환 (group_dm 아니면 에러). channel→realm 해석 포함.
async fn group_realm<S: Store + 'static>(
    st: &AppState<S>,
    channel_id: ChannelId,
) -> Result<domain::dm::RealmInfo, ApiError> {
    let channel = st.store.get(channel_id).await?.ok_or(ApiError::NotFound)?;
    let info = st.store.get_realm(channel.realm_id).await?.ok_or(ApiError::NotFound)?;
    if info.kind != RealmKind::GroupDm {
        return Err(ApiError::BadRequest("not a group DM channel".into()));
    }
    Ok(info)
}

/// 그룹DM 참가자 추가 (소유자만) → `CHANNEL_RECIPIENT_ADD` 팬아웃.
async fn add_recipient<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((channel_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let target = parse_user(&user_id)?;
    let info = group_realm(&st, cid).await?;
    if info.owner_id != Some(actor) {
        return Err(ApiError::Forbidden); // 소유자만 추가 (permissions.md §5).
    }
    let user = st.store.find_by_id(target).await?.ok_or(ApiError::BadRequest("recipient does not exist".into()))?;

    st.store.add_member(info.id, target).await?;
    let payload = recipient_add_payload(info.id, cid, &user);
    // 이벤트 소싱 사실(D48/E2): 그룹DM 참가자 추가 = Realm 멤버 합류 → MemberJoined(projection 멤버집합 정확).
    let fact = domain::event::RealmEventKind::MemberJoined { user: target };
    let _ = st.emitter.emit(info.id, "CHANNEL_RECIPIENT_ADD".into(), payload, Some(fact)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// 그룹DM 참가자 제거(소유자) 또는 본인 탈퇴 → `CHANNEL_RECIPIENT_REMOVE` 팬아웃. 소유자는 탈퇴 불가.
async fn remove_recipient<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((channel_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let target = parse_user(&user_id)?;
    let info = group_realm(&st, cid).await?;

    if info.owner_id == Some(target) {
        return Err(ApiError::BadRequest("owner cannot leave the group".into())); // 양도 후 가능(후속).
    }
    if target == actor {
        if !st.store.is_member(info.id, actor).await? {
            return Err(ApiError::NotFound);
        }
    } else if info.owner_id != Some(actor) {
        return Err(ApiError::Forbidden); // 타인 제거는 소유자만.
    }

    if !st.store.remove_member(info.id, target).await? {
        return Err(ApiError::NotFound);
    }
    let payload = recipient_remove_payload(info.id, cid, target);
    // 이벤트 소싱 사실(D48/E2): 그룹DM 참가자 제거/탈퇴 = Realm 멤버 이탈 → MemberLeft.
    let fact = domain::event::RealmEventKind::MemberLeft { user: target };
    let _ = st.emitter.emit(info.id, "CHANNEL_RECIPIENT_REMOVE".into(), payload, Some(fact)).await;
    Ok(StatusCode::NO_CONTENT)
}
