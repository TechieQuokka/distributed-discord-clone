//! Gateway 공유 상태 (개념: state). WS·메시지전송 라우트가 공유.
//!
//! Router/transport 제네릭(T)은 여기 격리(rest-api는 S만). 조합은 server에서.

use std::sync::Arc;

use auth::TokenKeys;
use domain::id::SnowflakeGenerator;
use domain::repo::Store;
use node::clock::Clock;
use node::{Presence, Router};
use transport::NodeTransport;

use crate::hub::Hub;

pub struct GatewayState<S: Store, T: NodeTransport> {
    pub router: Arc<Router<T>>,
    pub store: Arc<S>,
    pub keys: Arc<TokenKeys>,
    pub snowflakes: Arc<SnowflakeGenerator>,
    pub clock: Arc<dyn Clock>,
    pub hub: Hub,
    /// 전역 presence 레지스트리 (Q11/D12) — server가 소유, inbound gossip 루프와 공유.
    pub presence: Arc<Presence>,
    pub local_node_id: u64,
    /// 하트비트 권고 주기(ms) — HELLO로 클라에 안내.
    pub heartbeat_interval_ms: u64,
}

impl<S: Store, T: NodeTransport> Clone for GatewayState<S, T> {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            keys: Arc::clone(&self.keys),
            snowflakes: Arc::clone(&self.snowflakes),
            clock: Arc::clone(&self.clock),
            hub: self.hub.clone(),
            presence: Arc::clone(&self.presence),
            local_node_id: self.local_node_id,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
        }
    }
}
