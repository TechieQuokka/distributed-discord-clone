//! REST 라우트 조합 (개념: routes). 개념별 서브모듈을 하나의 Router로.

pub mod attachment;
pub mod audit;
pub mod auth;
pub mod channel;
pub mod dm;
pub mod guild;
pub mod invite;
pub mod member;
pub mod message;
pub mod read_state;
pub mod relationship;
pub mod role;
pub mod sync;
pub mod thread;
pub mod webauthn;
pub mod webhook;

use domain::repo::Store;

use crate::state::AppState;

/// 전체 REST 라우터 (state 주입 완료). Rate limit 미들웨어(D32)를 전 라우트에 적용.
pub fn router<S: Store + 'static>(state: AppState<S>) -> axum::Router {
    axum::Router::new()
        .merge(auth::routes::<S>())
        .merge(webauthn::routes::<S>())
        .merge(guild::routes::<S>())
        .merge(invite::routes::<S>())
        .merge(dm::routes::<S>())
        .merge(relationship::routes::<S>())
        .merge(read_state::routes::<S>())
        .merge(sync::routes::<S>())
        .merge(member::routes::<S>())
        .merge(role::routes::<S>())
        .merge(channel::routes::<S>())
        .merge(message::routes::<S>())
        .merge(thread::routes::<S>())
        .merge(attachment::routes::<S>())
        .merge(webhook::routes::<S>())
        .merge(audit::routes::<S>())
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::ratelimit::rate_limit::<S>))
        .with_state(state)
}
