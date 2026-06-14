//! REST 라우트 조합 (개념: routes). 개념별 서브모듈을 하나의 Router로.

pub mod auth;
pub mod channel;
pub mod guild;
pub mod invite;
pub mod message;
pub mod role;

use domain::repo::Store;

use crate::state::AppState;

/// 전체 REST 라우터 (state 주입 완료).
pub fn router<S: Store + 'static>(state: AppState<S>) -> axum::Router {
    axum::Router::new()
        .merge(auth::routes::<S>())
        .merge(guild::routes::<S>())
        .merge(invite::routes::<S>())
        .merge(role::routes::<S>())
        .merge(channel::routes::<S>())
        .merge(message::routes::<S>())
        .with_state(state)
}
