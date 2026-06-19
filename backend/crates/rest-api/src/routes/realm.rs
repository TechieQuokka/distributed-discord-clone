//! 내 Realm 목록 라우트 (개념: routes/realm). `GET /users/@me/realms`.
//!
//! 웹 UI 첫 화면(서버/DM 목록)을 그리기 위한 읽기 엔드포인트. CLI는 id를 인자로 직접
//! 넘겨 불필요했으나, 브라우저 클라이언트는 로그인 직후 가입한 realm을 이름·종류와 함께
//! 열거해야 한다. 기존 `member_realm_ids` + `get_realm`을 조합할 뿐(저장소 무변경).

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use domain::repo::Store;
use serde::Serialize;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new().route("/users/@me/realms", get(list_realms::<S>))
}

#[derive(Serialize)]
pub struct RealmView {
    pub id: String,
    /// guild/dm/group_dm. 클라가 서버 vs DM 분기.
    pub kind: String,
    /// 길드/그룹DM 이름. 1:1 DM은 null(상대 유저로 표시).
    pub name: Option<String>,
    /// 그룹DM 소유자(멤버 관리 권한). 길드/1:1 DM은 null.
    pub owner_id: Option<String>,
}

/// 내가 멤버인 Realm 목록(길드 + DM + 그룹DM). READY의 realm id 목록과 동치이나 이름·종류 포함.
async fn list_realms<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<RealmView>>, ApiError> {
    let ids = st.store.member_realm_ids(user).await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        // 멤버 테이블엔 있으나 realm 행이 사라진 경우(이론상) 조용히 스킵.
        if let Some(info) = st.store.get_realm(id).await? {
            out.push(RealmView {
                id: info.id.0.raw().to_string(),
                kind: info.kind.as_str().to_owned(),
                name: info.name,
                owner_id: info.owner_id.map(|o| o.0.raw().to_string()),
            });
        }
    }
    Ok(Json(out))
}
