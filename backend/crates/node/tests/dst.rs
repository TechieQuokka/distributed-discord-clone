//! DST 하네스 (D25) — SimTransport + SimClock(ManualClock) + 시드 RNG로 멀티노드 클러스터를
//! 단일 프로세스·가상 시간에서 결정론적으로 구동. 카오스(지연/유실/파티션)는 SimNetwork가 주입.
//!
//! 여기서 검증하는 것:
//! 1. 크로스노드 구독+팬아웃이 **가상 시간**으로 동작 — 지연이 흐르기 전엔 배달 안 됨.
//! 2. **재현성** — 동일 시드 → 동일 메시지 id + 동일 배달 결과.
//! 3. **파티션** — 격리된 노드로의 팬아웃은 유실(소유 노드는 정상).
//!
//! 액터는 tokio에서 돌지만, 노드 간 배달은 SimNetwork가 가상 시간에 스케줄하므로 네트워크
//! 경로는 결정론적이다. 시계(ManualClock)를 고정해 Snowflake id도 재현 가능.

use std::sync::Arc;

use domain::id::{ChannelId, EPOCH_MS, RealmId, Snowflake, SnowflakeGenerator, UserId};
use node::clock::{Clock, ManualClock};
use node::{HashRing, LocalDelivery, RealmEvent, Router};
use tokio::sync::mpsc;
use transport::{SimConfig, SimNetwork, SimTransport};

fn uid(n: u64) -> UserId {
    UserId(Snowflake::from_raw(n))
}
fn ring_2() -> HashRing {
    let mut r = HashRing::new(100);
    r.add_node(1);
    r.add_node(2);
    r
}
fn first_realm_owned_by(ring: &HashRing, node: u64) -> RealmId {
    (0u64..)
        .map(|x| RealmId(Snowflake::from_raw(x)))
        .find(|r| ring.owner(r.0.raw()) == Some(node))
        .unwrap()
}

/// 가상 시계를 `to`까지 진행시키고, 양 노드의 도착 메시지를 `handle_inbound`로 처리.
/// 노드2의 LocalDelivery(팬아웃 배달)를 수집해 반환.
async fn drive(
    net: &SimNetwork,
    to: u64,
    r1: &Router<SimTransport>,
    r2: &Router<SimTransport>,
) -> Vec<LocalDelivery> {
    net.advance_to(to);
    let mut delivered = Vec::new();
    for ib in net.take_inbound(1) {
        let _ = r1.handle_inbound(ib).await;
    }
    for ib in net.take_inbound(2) {
        if let Ok(Some(d)) = r2.handle_inbound(ib).await {
            delivered.push(d);
        }
    }
    delivered
}

/// 1회 시나리오: 시드/카오스/파티션 주입 → (메시지 id, 노드2에 배달된 내용들).
async fn scenario(seed: u64, cfg: SimConfig, partition_node2: bool) -> (u64, Vec<String>) {
    let net = SimNetwork::new(seed, cfg);
    let t1 = net.transport(1);
    let t2 = net.transport(2);
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(EPOCH_MS + 1)); // SimClock(고정)

    let ring = ring_2();
    let realm = first_realm_owned_by(&ring, 1); // 노드1 소유
    let (etx1, mut erx1) = mpsc::channel(64);
    let (etx2, _erx2) = mpsc::channel(64);
    let r1 = Router::new(1, Arc::new(SnowflakeGenerator::new(1)), Arc::clone(&clock), ring_2(), t1, etx1);
    let r2 = Router::new(2, Arc::new(SnowflakeGenerator::new(2)), Arc::clone(&clock), ring, t2, etx2);

    // A@노드1(소유) 로컬 구독, B@노드2 구독 → 소유 노드1로 sim 포워딩.
    r1.route_subscribe(realm, uid(0xA), 1).await.unwrap();
    r2.route_subscribe(realm, uid(0xB), 2).await.unwrap();

    if partition_node2 {
        net.partition(2);
    }

    // 포워딩된 Subscribe가 노드1에 도착하도록 가상 시간 진행.
    drive(&net, 100, &r1, &r2).await;

    // A가 노드1에서 전송 → 액터가 id·이벤트 확정.
    let chan = ChannelId(Snowflake::from_raw(0xC0));
    let mid = r1.route_send_local(realm, chan, uid(0xA), "dst-msg".into(), None).await.unwrap();

    // 이벤트 → 팬아웃 (노드2로 RealmFanout sim 전송).
    let event = erx1.recv().await.unwrap();
    let RealmEvent::MessageCreated { .. } = &event;
    r1.fanout(event).await.unwrap();

    // 팬아웃이 노드2에 도착하도록 진행 + 수집.
    let delivered = drive(&net, 1000, &r1, &r2).await;
    let contents: Vec<String> = delivered.into_iter().flat_map(|d| {
        d.user_ids.iter().map(move |_| d.content.clone()).collect::<Vec<_>>()
    }).collect();
    (mid.0.raw(), contents)
}

/// 지연이 있는 정상 경로: 동일 시드 2회 → 완전히 동일한 결과 (재현성, D25).
#[tokio::test]
async fn deterministic_replay_same_seed() {
    let cfg = SimConfig { min_latency_ms: 5, max_latency_ms: 30, drop_prob: 0.0 };
    let a = scenario(99, cfg.clone(), false).await;
    let b = scenario(99, cfg, false).await;
    assert_eq!(a, b, "동일 시드는 동일 메시지 id + 동일 배달이어야 함");
    // 노드2의 B에게 메시지가 배달됨.
    assert_eq!(a.1, vec!["dst-msg".to_string()]);
}

/// 파티션: 노드2 격리 → 팬아웃이 노드2에 닿지 못해 B는 못 받음. (소유 노드는 정상 처리.)
#[tokio::test]
async fn partition_drops_fanout_to_isolated_node() {
    let cfg = SimConfig { min_latency_ms: 1, max_latency_ms: 1, drop_prob: 0.0 };
    let (_mid, contents) = scenario(7, cfg, true).await;
    assert!(contents.is_empty(), "격리된 노드2로의 팬아웃은 유실되어야 함");
}
