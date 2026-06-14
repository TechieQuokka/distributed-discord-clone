//! REST 공유 상태 (개념: state). store·키·ID generator·clock 주입.
//!
//! 모든 리포지토리는 `Store` 한 타입에 통합 → **제네릭 `AppState<S>`** 하나로 주입
//! (repo trait가 RPITIT라 dyn 불가하나, Store 통합으로 제네릭 폭발은 회피).
//! Snowflake generator는 노드당 1개를 server가 소유해 주입(D11) — Router와 동일 인스턴스.

use std::sync::Arc;

use auth::TokenKeys;
use domain::id::SnowflakeGenerator;
use domain::repo::Store;
use node::clock::Clock;

pub struct AppState<S: Store> {
    pub store: Arc<S>,
    pub keys: Arc<TokenKeys>,
    pub snowflakes: Arc<SnowflakeGenerator>,
    pub clock: Arc<dyn Clock>,
}

// Arc만 복제 (derive(Clone)은 S:Clone를 요구하므로 수동 구현).
impl<S: Store> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            keys: Arc::clone(&self.keys),
            snowflakes: Arc::clone(&self.snowflakes),
            clock: Arc::clone(&self.clock),
        }
    }
}

impl<S: Store> AppState<S> {
    pub fn new(
        store: Arc<S>,
        keys: Arc<TokenKeys>,
        snowflakes: Arc<SnowflakeGenerator>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { store, keys, snowflakes, clock }
    }

    /// 현재 시각(unix seconds) — refresh 만료/검증용.
    pub fn now_unix(&self) -> i64 {
        (self.clock.now_ms() / 1000) as i64
    }
}
