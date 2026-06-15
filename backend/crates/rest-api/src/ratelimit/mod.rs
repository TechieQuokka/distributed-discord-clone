//! Rate limiting (개념: ratelimit). D18/D32 — Token Bucket, **per-node 인메모리**(DB-D5 휘발).
//!
//! 분산 근사(D32): 각 노드가 자기 버킷을 로컬 보유 → 노드 3~10개에선 누수 미미. 정밀이 필요한
//! 엔드포인트만 후속에 유저-해시 소유 노드로 승격(b). 현재는 REST 엣지에 미들웨어로 적용:
//! - `/auth/*` = 노드 전역 버킷(가입/로그인 봇방지, PoW(D18)와 상보적)
//! - 그 외 = **인증 유저별** 버킷(토큰 검증 성공 시), 미인증은 전역 anon 버킷
//!
//! 429 응답 + `X-RateLimit-{Limit,Remaining,Reset}` / `Retry-After` 헤더(rest.md §0).

use std::collections::HashMap;
use std::sync::Mutex;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use domain::repo::Store;
use serde_json::json;

use crate::state::AppState;

/// 토큰 버킷 (순수 상태). `tokens`는 연속값(부분 토큰 누적), `last_ms`는 마지막 리필 시각.
#[derive(Clone, Copy)]
struct TokenBucket {
    tokens: f64,
    last_ms: u64,
}

impl TokenBucket {
    fn new(capacity: f64, now_ms: u64) -> Self {
        Self { tokens: capacity, last_ms: now_ms }
    }

    /// 경과 시간만큼 리필(상한 capacity).
    fn refill(&mut self, now_ms: u64, capacity: f64, refill_per_sec: f64) {
        if now_ms > self.last_ms {
            let elapsed_s = (now_ms - self.last_ms) as f64 / 1000.0;
            self.tokens = (self.tokens + elapsed_s * refill_per_sec).min(capacity);
            self.last_ms = now_ms;
        }
    }

    /// 토큰 1개 소비 시도. 리필 후 ≥1이면 차감하고 true.
    fn try_take(&mut self, now_ms: u64, capacity: f64, refill_per_sec: f64) -> bool {
        self.refill(now_ms, capacity, refill_per_sec);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// 한 버킷 클래스의 규칙: 용량 + 초당 리필.
#[derive(Clone, Copy)]
pub struct RateRule {
    pub capacity: f64,
    pub refill_per_sec: f64,
}

/// 한 번의 판정 결과 (헤더 구성용).
pub struct Outcome {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub retry_after_secs: u64,
    pub reset_unix: u64,
}

/// per-node Token Bucket 레지스트리 (D32). `rule:identity` 키로 버킷을 들고 판정한다.
pub struct RateLimiter {
    rules: HashMap<&'static str, RateRule>,
    fallback: RateRule,
    buckets: Mutex<HashMap<String, TokenBucket>>,
}

impl RateLimiter {
    pub fn from_rules(rules: HashMap<&'static str, RateRule>, fallback: RateRule) -> Self {
        Self { rules, fallback, buckets: Mutex::new(HashMap::new()) }
    }

    /// 운영 기본값. 정상 사용엔 넉넉하고 폭주(봇)엔 걸리는 수준.
    pub fn with_defaults() -> Self {
        let mut rules = HashMap::new();
        rules.insert("auth", RateRule { capacity: 20.0, refill_per_sec: 5.0 }); // 가입/로그인
        rules.insert("user", RateRule { capacity: 120.0, refill_per_sec: 60.0 }); // 인증 유저
        rules.insert("anon", RateRule { capacity: 60.0, refill_per_sec: 30.0 }); // 미인증 기타
        Self::from_rules(rules, RateRule { capacity: 60.0, refill_per_sec: 30.0 })
    }

    /// 사실상 무제한(테스트/내부 호출용) — 기존 흐름·통합테스트가 한도에 안 걸리게.
    pub fn lenient() -> Self {
        let r = RateRule { capacity: 1e9, refill_per_sec: 1e9 };
        Self::from_rules(HashMap::new(), r)
    }

    fn rule(&self, name: &str) -> RateRule {
        self.rules.get(name).copied().unwrap_or(self.fallback)
    }

    /// `rule` 클래스 + `identity`의 버킷에서 토큰 1개 소비 시도. 휘발 per-node(DB-D5).
    pub fn check(&self, rule_name: &str, identity: &str, now_ms: u64) -> Outcome {
        let rule = self.rule(rule_name);
        let key = format!("{rule_name}:{identity}");
        let mut map = self.buckets.lock().unwrap();
        let bucket = map.entry(key).or_insert_with(|| TokenBucket::new(rule.capacity, now_ms));
        let allowed = bucket.try_take(now_ms, rule.capacity, rule.refill_per_sec);

        let now_unix = now_ms / 1000;
        let deficit_to_full = (rule.capacity - bucket.tokens).max(0.0);
        let reset_unix = now_unix + (deficit_to_full / rule.refill_per_sec).ceil() as u64;
        let retry_after_secs = if allowed {
            0
        } else {
            ((1.0 - bucket.tokens) / rule.refill_per_sec).ceil().max(1.0) as u64
        };
        Outcome {
            allowed,
            limit: rule.capacity as u64,
            remaining: bucket.tokens.floor() as u64,
            retry_after_secs,
            reset_unix,
        }
    }
}

/// path + 인증 여부로 버킷 클래스와 식별자를 정한다.
fn classify(path: &str, uid: Option<u64>) -> (&'static str, String) {
    if path.starts_with("/auth/") {
        ("auth", "global".to_string()) // 가입/로그인은 노드 전역 버킷(미인증)
    } else if let Some(u) = uid {
        ("user", u.to_string()) // 인증 유저별
    } else {
        ("anon", "global".to_string())
    }
}

fn hv(n: u64) -> HeaderValue {
    HeaderValue::from_str(&n.to_string()).unwrap_or(HeaderValue::from_static("0"))
}

fn apply_headers(h: &mut HeaderMap, o: &Outcome) {
    h.insert(HeaderName::from_static("x-ratelimit-limit"), hv(o.limit));
    h.insert(HeaderName::from_static("x-ratelimit-remaining"), hv(o.remaining));
    h.insert(HeaderName::from_static("x-ratelimit-reset"), hv(o.reset_unix));
}

/// Rate limit 미들웨어 (REST 라우터에 적용). 초과 시 429 + 헤더, 통과 시 응답에 잔량 헤더.
pub async fn rate_limit<S: Store + 'static>(
    State(st): State<AppState<S>>,
    req: Request,
    next: Next,
) -> Response {
    // 인증 토큰이 유효하면 유저별, 아니면 전역.
    let uid = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .and_then(|t| st.keys.verify_access(t).ok());
    let (rule, id) = classify(req.uri().path(), uid);
    let outcome = st.ratelimit.check(rule, &id, st.clock.now_ms());

    if !outcome.allowed {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "rate_limited", "message": "too many requests" })),
        )
            .into_response();
        apply_headers(resp.headers_mut(), &outcome);
        resp.headers_mut().insert(RETRY_AFTER, hv(outcome.retry_after_secs));
        return resp;
    }

    let mut resp = next.run(req).await;
    apply_headers(resp.headers_mut(), &outcome);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_denies_after_capacity_then_refills() {
        let rules = {
            let mut m = HashMap::new();
            m.insert("t", RateRule { capacity: 3.0, refill_per_sec: 1.0 });
            m
        };
        let rl = RateLimiter::from_rules(rules, RateRule { capacity: 3.0, refill_per_sec: 1.0 });
        // 같은 시각에 3개 통과, 4번째 거부.
        assert!(rl.check("t", "u1", 1000).allowed);
        assert!(rl.check("t", "u1", 1000).allowed);
        assert!(rl.check("t", "u1", 1000).allowed);
        let denied = rl.check("t", "u1", 1000);
        assert!(!denied.allowed);
        assert_eq!(denied.remaining, 0);
        assert!(denied.retry_after_secs >= 1);
        // 1초 뒤 토큰 1개 리필 → 다시 통과.
        assert!(rl.check("t", "u1", 2000).allowed);
    }

    #[test]
    fn identities_have_independent_buckets() {
        let rl = RateLimiter::from_rules(
            HashMap::new(),
            RateRule { capacity: 1.0, refill_per_sec: 1.0 },
        );
        assert!(rl.check("x", "alice", 0).allowed);
        assert!(!rl.check("x", "alice", 0).allowed, "alice 소진");
        assert!(rl.check("x", "bob", 0).allowed, "bob은 독립 버킷");
    }

    #[test]
    fn classify_routes_auth_user_anon() {
        assert_eq!(classify("/auth/login", None).0, "auth");
        assert_eq!(classify("/auth/login", Some(7)).0, "auth"); // 인증돼도 auth 경로는 전역
        assert_eq!(classify("/guilds", Some(7)), ("user", "7".to_string()));
        assert_eq!(classify("/guilds", None), ("anon", "global".to_string()));
    }

    #[test]
    fn lenient_never_denies() {
        let rl = RateLimiter::lenient();
        for i in 0..10_000 {
            assert!(rl.check("anything", "u", i).allowed);
        }
    }
}
