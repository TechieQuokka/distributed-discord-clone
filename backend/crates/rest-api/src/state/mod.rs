//! REST 공유 상태 (개념: state). store·키·ID generator·clock 주입.
//!
//! 모든 리포지토리는 `Store` 한 타입에 통합 → **제네릭 `AppState<S>`** 하나로 주입
//! (repo trait가 RPITIT라 dyn 불가하나, Store 통합으로 제네릭 폭발은 회피).
//! Snowflake generator는 노드당 1개를 server가 소유해 주입(D11) — Router와 동일 인스턴스.

use std::sync::Arc;

use auth::{PowKeys, TokenKeys};
use domain::emit::{RealmEmitter, UserEmitter};
use domain::id::SnowflakeGenerator;
use domain::repo::Store;
use node::clock::Clock;

use crate::ratelimit::RateLimiter;

pub struct AppState<S: Store> {
    pub store: Arc<S>,
    pub keys: Arc<TokenKeys>,
    /// 가입 봇방지 PoW 챌린지 키 (D18). 멀티노드 공유(`POW_SECRET`).
    pub pow: Arc<PowKeys>,
    /// per-node Rate limiter (D32, 휘발 DB-D5). REST 미들웨어가 사용.
    pub ratelimit: Arc<RateLimiter>,
    pub snowflakes: Arc<SnowflakeGenerator>,
    pub clock: Arc<dyn Clock>,
    /// Realm 단위 실시간 이벤트 포트 (D39) — 멤버 변동 등을 구독자표로 팬아웃. server가 Router를 주입.
    pub emitter: Arc<dyn RealmEmitter>,
    /// 유저 단위 실시간 이벤트 포트 — 친구·차단 등 Realm 무관 이벤트. server가 Hub를 주입.
    pub user_emitter: Arc<dyn UserEmitter>,
}

// Arc만 복제 (derive(Clone)은 S:Clone를 요구하므로 수동 구현).
impl<S: Store> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            keys: Arc::clone(&self.keys),
            pow: Arc::clone(&self.pow),
            ratelimit: Arc::clone(&self.ratelimit),
            snowflakes: Arc::clone(&self.snowflakes),
            clock: Arc::clone(&self.clock),
            emitter: Arc::clone(&self.emitter),
            user_emitter: Arc::clone(&self.user_emitter),
        }
    }
}

impl<S: Store> AppState<S> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<S>,
        keys: Arc<TokenKeys>,
        pow: Arc<PowKeys>,
        ratelimit: Arc<RateLimiter>,
        snowflakes: Arc<SnowflakeGenerator>,
        clock: Arc<dyn Clock>,
        emitter: Arc<dyn RealmEmitter>,
        user_emitter: Arc<dyn UserEmitter>,
    ) -> Self {
        Self { store, keys, pow, ratelimit, snowflakes, clock, emitter, user_emitter }
    }

    /// 현재 시각(unix seconds) — refresh 만료/검증용.
    pub fn now_unix(&self) -> i64 {
        (self.clock.now_ms() / 1000) as i64
    }
}
