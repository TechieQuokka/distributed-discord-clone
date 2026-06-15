//! Realm 이벤트 emit 포트 (개념: emit). D39 — 비-메시지 실시간 이벤트 팬아웃.
//!
//! REST 등 엣지가 멤버 변동 같은 이벤트를 **구독자표(D12)**로 흘려보내기 위한 포트.
//! 구현(adapter)은 `node::Router`(로컬 소유 액터 / 원격 위임). domain은 IO를 모른다(P2):
//! `payload`는 클라에 그대로 나갈 JSON을 **미리 직렬화한 불투명 문자열** — 하위 계층은 파싱하지 않고
//! 통과시키고, 최종 배달 직전 gateway가 1회 역파싱한다(JSON 단일 출처 = 생산 엣지, D39).
//!
//! repo 포트(RPITIT)와 달리 여기선 `dyn` 주입이 필요해(엣지가 transport 제네릭을 모름) **박스 future**를 쓴다.

use core::future::Future;
use core::pin::Pin;

use crate::id::{RealmId, UserId};

/// `Send` 박스 future 별칭 (dyn 호환).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Realm 실시간 이벤트 emit 포트 (D39). 구독자표(D12)를 타는 **Realm 단위** 팬아웃.
pub trait RealmEmitter: Send + Sync {
    /// `t` = DISPATCH 이벤트 이름(`"GUILD_MEMBER_ADD"` 등), `payload` = 직렬화된 JSON.
    /// fire-and-forget — 팬아웃은 소유 노드 액터를 거쳐 비동기로 흐른다.
    fn emit(&self, realm: RealmId, t: String, payload: String) -> BoxFuture<'_, ()>;
}

/// **유저 단위** 실시간 이벤트 emit 포트 (친구·차단 등 Realm 무관 이벤트).
///
/// Realm 팬아웃(D12)과 분리되는 전역 유저 상태 경로(D12 §"팬아웃과 전역 presence 분리").
/// 구현(adapter)은 gateway `Hub` — 대상 유저의 **이 노드 로컬 세션**에 배달한다.
/// ⚠ 크로스노드 유저 라우팅(다른 노드의 세션에 배달)은 전역 presence/gossip(Q11) 도입 후의 seam.
pub trait UserEmitter: Send + Sync {
    /// `users`에게 `t`/`payload`(직렬화된 JSON)를 배달. fire-and-forget.
    fn emit_to_users(&self, users: &[UserId], t: String, payload: String) -> BoxFuture<'_, ()>;
}
