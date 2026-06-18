//! `server` — REST(auth/guild/history) + Gateway(WS) + node 조립, 실행 진입점.
//! Phase 1: 단일노드 실시간 메시징. Phase 2: raw-TCP+mTLS 멀티노드(CLUSTER_CONFIG 지정 시).

use std::sync::Arc;

use auth::TokenKeys;
use cluster_config::ClusterConfig;
use domain::id::SnowflakeGenerator;
use gateway::{GatewayState, Hub};
use node::clock::{Clock, SystemClock};
use node::{HashRing, Router};
use rest_api::AppState;
use storage::PgStore;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;
use transport::{Inbound, TcpTransport, TlsMaterial};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dev 유틸: `server gen-certs <out_dir> <id1> <id2> ...` — 공유 CA + 노드별 cert(SAN=127.0.0.1) 파일 생성.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("gen-certs") {
        return gen_certs(&args[2..]);
    }
    if args.get(1).map(String::as_str) == Some("gen-keys") {
        let (sk, pk) = TokenKeys::generate()?.export_hex();
        println!("PASETO_SECRET={sk}");
        println!("PASETO_PUBLIC={pk}");
        // PoW 챌린지 서명 키 (D18) — 멀티노드는 공유 필수(PASETO와 동일).
        println!("POW_SECRET={}", auth::PowKeys::generate()?.export_hex());
        return Ok(());
    }

    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    transport::init_crypto(); // rustls ring provider.

    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL 미설정 — backend/.env 확인")?;
    let rest_addr = std::env::var("REST_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());

    // 멀티노드: CLUSTER_CONFIG(TOML) 지정 시 클러스터 설정 로드. 없으면 단일노드 기본.
    let cluster = match std::env::var("CLUSTER_CONFIG") {
        Ok(path) => Some(ClusterConfig::from_file(path)?),
        Err(_) => None,
    };
    let node_id: u64 = cluster.as_ref().map(|c| c.node.id).unwrap_or_else(|| env_parse("NODE_ID", 1));
    let worker_id: u16 =
        cluster.as_ref().map(|c| c.node.worker_id).unwrap_or_else(|| env_parse("NODE_WORKER_ID", 1));

    // 인프라.
    let pool = storage::connect(&database_url).await?;
    storage::run_migrations(&pool).await?;
    info!("db connected + migrations applied");

    let store = Arc::new(PgStore::new(pool.clone()));
    // 신규 월 메시지 파티션 사전 생성(D28, 04 §6) — 이번 달 + 2개월. 멱등(이미 있으면 0).
    // 다가오는 달 메시지가 DEFAULT로 새어 "최근=핫" 지역성이 무너지는 것을 방지.
    match store.ensure_message_partitions(2).await {
        Ok(n) => info!(created = n, "message partitions ensured (this month + 2)"),
        Err(e) => tracing::warn!(error = %e, "파티션 사전 생성 실패(계속 — DEFAULT가 흡수)"),
    }
    // 노드당 단일 Snowflake generator (D11) — Router·REST·Gateway 공유.
    let snowflakes = Arc::new(SnowflakeGenerator::new(worker_id));
    // PASETO 키: 멀티노드는 모든 노드가 **공유 키**여야 토큰 상호 검증 가능(D14).
    // env(PASETO_SECRET/PUBLIC) 있으면 로드, 없으면 생성(단일노드). `server gen-keys`로 생성.
    let keys = Arc::new(match (std::env::var("PASETO_SECRET"), std::env::var("PASETO_PUBLIC")) {
        (Ok(sk), Ok(pk)) => TokenKeys::import_hex(&sk, &pk)?,
        _ => {
            if cluster.is_some() {
                tracing::warn!("멀티노드인데 PASETO_SECRET/PUBLIC 미지정 — 노드 간 토큰 검증 실패함");
            }
            TokenKeys::generate()?
        }
    });
    // PoW 챌린지 키 (D18): 멀티노드는 모든 노드가 **공유 키**(POW_SECRET)여야 한 노드 발급 챌린지를
    // 다른 노드가 검증 가능(stateless, DB-D5). 없으면 생성(단일노드는 발급/검증이 같은 프로세스).
    let pow = Arc::new(match std::env::var("POW_SECRET") {
        Ok(s) => auth::PowKeys::import_hex(&s)?,
        Err(_) => {
            if cluster.is_some() {
                tracing::warn!("멀티노드인데 POW_SECRET 미지정 — 노드 간 PoW 챌린지 검증 실패 가능");
            }
            auth::PowKeys::generate()?
        }
    });
    // per-node Rate limiter (D32, 휘발 DB-D5) — 노드마다 독립 버킷(분산 근사). 기본 규칙 적용.
    let ratelimit = Arc::new(rest_api::RateLimiter::with_defaults());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    // 첨부 바이트 저장소 (D37) — 로컬 FS. env ATTACHMENT_DIR(기본 ./attachments).
    let attach_dir = std::env::var("ATTACHMENT_DIR").unwrap_or_else(|_| "./attachments".into());
    let blobs: Arc<dyn domain::blob::BlobStore> = Arc::new(storage::LocalFsBlobStore::new(&attach_dir)?);

    // 분산 코어: 링(클러스터의 모든 노드) + raw-TCP+mTLS 전송(D3/D16).
    let mut ring = HashRing::new(128);
    match &cluster {
        Some(c) => {
            ring.add_node(c.node.id);
            for p in &c.peers {
                ring.add_node(p.id);
            }
        }
        None => ring.add_node(node_id),
    }
    let transport = TcpTransport::new(node_id);
    let (events_tx, events_rx) = mpsc::channel(1024);
    let router = Arc::new(Router::new(
        node_id,
        Arc::clone(&snowflakes),
        Arc::clone(&clock),
        ring,
        transport.clone(),
        events_tx,
    ));
    let hub = Hub::new();
    // 전역 presence 레지스트리 (Q11/D12, 휘발 DB-D5) — gateway 세션 훅 + inbound gossip 루프가 공유.
    let presence = Arc::new(node::Presence::new());

    // persist-then-fanout 드라이버 (D24).
    tokio::spawn(gateway::run_dispatch(events_rx, Arc::clone(&router), Arc::clone(&store), hub.clone()));

    // 멀티노드 전송 가동: listen + dial + SWIM 멤버십(D45) + 크로스노드 inbound 루프.
    if let Some(c) = &cluster {
        let tls = load_tls(node_id)?;
        let cli_cfg = transport::client_config(&tls)?; // 동적 dial 재사용용 클라 설정.
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(1024);
        transport
            .listen(&c.node.listen_addr, transport::server_config(&tls)?, inbound_tx.clone())
            .await?;

        // SWIM 멤버십 뷰 (D45) — config peers를 seed로 주입. 파라미터는 env로 운영 노출(H4).
        let (swim_cfg, swim_interval_ms) = swim_config_from_env();
        let swim = Arc::new(node::Swim::new(
            node_id,
            c.node.listen_addr.clone(),
            swim_cfg,
            node_id.wrapping_mul(0x9E37_79B9_7F4A_7C15), // 노드별 시드.
        ));
        for p in &c.peers {
            swim.seed_member(p.id, p.addr.clone());
        }

        // 신규 노드 Alive 학습 시 훅(D45/D46): (1) 작은 id가 큰 id에게 동적 dial, (2) presence 스냅샷 push.
        let on_member_up: node::MemberUpHook = {
            let transport = transport.clone();
            let inbound = inbound_tx.clone();
            let cli_cfg = cli_cfg.clone();
            let presence = Arc::clone(&presence);
            let router = Arc::clone(&router);
            let local = node_id;
            Arc::new(move |peer: u64, addr: String| {
                if local < peer {
                    let host = host_of(&addr);
                    transport.connect_peer(peer, addr, host, cli_cfg.clone(), inbound.clone());
                }
                // D46 presence anti-entropy: 내 호스팅 유저 presence를 신규 노드에 push.
                let snap = presence.snapshot_local(local);
                if !snap.is_empty() {
                    let router = Arc::clone(&router);
                    tokio::spawn(async move {
                        for (user, status) in snap {
                            router
                                .send_to(
                                    peer,
                                    protocol::NodeMessage::PresenceGossip {
                                        user_id: user,
                                        node_id: local,
                                        status: status.as_u8(),
                                    },
                                )
                                .await;
                        }
                    });
                }
            })
        };

        // 부트스트랩 dial: dynamic이면 seed 전체에 무조건(합류 연결), 아니면 정적 풀메시(작은→큰 id).
        if c.node.dynamic {
            for p in &c.peers {
                transport.connect_peer(p.id, p.addr.clone(), host_of(&p.addr), cli_cfg.clone(), inbound_tx.clone());
            }
        } else {
            for peer in c.peers_to_dial() {
                transport.connect_peer(peer.id, peer.addr.clone(), host_of(&peer.addr), cli_cfg.clone(), inbound_tx.clone());
            }
        }

        // 수신: 원격 RealmSend/Fanout / PresenceGossip / UserDeliver / **SWIM 멤버십**.
        tokio::spawn(run_inbound(
            inbound_rx,
            Arc::clone(&router),
            hub.clone(),
            Arc::clone(&clock),
            Arc::clone(&presence),
            Arc::clone(&store),
            Arc::clone(&swim),
            on_member_up.clone(),
        ));

        // SWIM 주기 드라이버 (D45) — 정적 run_failure_detector(D23)를 대체. dynamic이면 seed에 SwimJoin.
        let seeds: Vec<u64> = if c.node.dynamic { c.peers.iter().map(|p| p.id).collect() } else { Vec::new() };
        tokio::spawn(node::run_swim(
            Arc::clone(&swim),
            Arc::clone(&router),
            Arc::clone(&clock),
            on_member_up,
            swim_interval_ms,
            seeds,
        ));
        info!(node_id, listen = %c.node.listen_addr, peers = c.peers.len(), dynamic = c.node.dynamic, "node mesh active (mTLS) + SWIM membership");
    } else {
        info!(node_id, "single-node mode (no cluster config)");
    }

    // WebAuthn/Passkeys (D19) — RP가 env로 구성된 경우만 활성(없으면 webauthn 엔드포인트 404).
    let webauthn = match (std::env::var("WEBAUTHN_RP_ID"), std::env::var("WEBAUTHN_RP_ORIGIN")) {
        (Ok(rp_id), Ok(rp_origin)) => match auth::WebauthnService::new(&rp_id, &rp_origin) {
            Ok(s) => {
                info!(rp_id, rp_origin, "webauthn enabled");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::warn!(error = %e, "WEBAUTHN 설정 무효 — 비활성");
                None
            }
        },
        _ => None,
    };

    // REST(auth/guild/history) 라우터. Router를 Realm emit 포트(D39, 멤버 변동 팬아웃),
    // UserRouter를 유저 emit 포트(친구·차단·읽음 통지)로 주입 — 크로스노드 타깃 배달(D43).
    let user_router = gateway::UserRouter::new(hub.clone(), Arc::clone(&presence), Arc::clone(&router), node_id);
    let rest = rest_api::router(AppState::new(
        Arc::clone(&store),
        Arc::clone(&keys),
        Arc::clone(&pow),
        Arc::clone(&ratelimit),
        Arc::clone(&snowflakes),
        Arc::clone(&clock),
        Arc::clone(&router) as Arc<dyn domain::emit::RealmEmitter>,
        Arc::new(user_router) as Arc<dyn domain::emit::UserEmitter>,
        Arc::clone(&blobs),
        webauthn,
    ));

    // Gateway(WS + 메시지 전송) 라우터.
    let gw = gateway::router(GatewayState {
        router: Arc::clone(&router),
        store: Arc::clone(&store),
        keys: Arc::clone(&keys),
        snowflakes: Arc::clone(&snowflakes),
        clock: Arc::clone(&clock),
        hub,
        presence: Arc::clone(&presence),
        local_node_id: node_id,
        heartbeat_interval_ms: 30_000,
    });

    let app = rest.merge(gw);
    let listener = tokio::net::TcpListener::bind(&rest_addr).await?;
    info!(addr = %rest_addr, node_id, worker_id, "listening: REST /auth /guilds /channels + WS /gateway");
    axum::serve(listener, app).await?;
    Ok(())
}

/// 원격 노드 메시지 처리 루프 (Phase 2 크로스노드 배달 + Phase 5 SWIM).
/// 모든 인바운드는 그 자체로 피어 liveness 증거 → `record_seen`으로 생사 뷰 갱신 (D23).
#[allow(clippy::too_many_arguments)]
async fn run_inbound(
    mut rx: mpsc::Receiver<Inbound>,
    router: Arc<Router<TcpTransport>>,
    hub: Hub,
    clock: Arc<dyn Clock>,
    presence: Arc<node::Presence>,
    store: Arc<PgStore>,
    swim: Arc<node::Swim>,
    on_member_up: node::MemberUpHook,
) {
    use protocol::NodeMessage as M;
    while let Some(inbound) = rx.recv().await {
        let now = clock.now_ms();
        let src = inbound.src;
        router.membership().record_seen(src, now);
        match inbound.msg {
            // 전역 presence gossip(Q11/D12): view 갱신 + 그 유저의 로컬 친구에게 PRESENCE_UPDATE 배달.
            M::PresenceGossip { user_id, node_id, status } => {
                gateway::presence::apply_gossip(&presence, &hub, &*store, user_id, node_id, status).await;
            }
            // 크로스노드 유저 이벤트 타깃 배달(D43): 이 노드의 로컬 세션에 흘린다.
            M::UserDeliver { t, payload, user_ids } => {
                gateway::deliver_user(&hub, t, payload, &user_ids).await;
            }
            // 크로스노드 RESUME(D24): 원조 노드측 — 세션 export(검증+제거) → 요청 노드에 ResumeState 회신.
            M::ResumeFetch { session_id, token, last_seq, requester } => {
                match hub.export_migration(session_id, &token, last_seq) {
                    gateway::MigrationExport::NotHere => {} // 다른 노드가 원조 — 무응답.
                    gateway::MigrationExport::Reject => {
                        router
                            .send_to(requester, M::ResumeState {
                                session_id,
                                found: false,
                                user_id: 0,
                                last_seq: 0,
                                resume_token: String::new(),
                                frames: Vec::new(),
                            })
                            .await;
                    }
                    gateway::MigrationExport::Ok { user_id, last_seq, resume_token, frames } => {
                        router
                            .send_to(requester, M::ResumeState {
                                session_id,
                                found: true,
                                user_id,
                                last_seq,
                                resume_token,
                                frames,
                            })
                            .await;
                    }
                }
            }
            // 크로스노드 RESUME(D24): 요청 노드측 — 원조 응답을 대기 중 핸들러에 전달.
            M::ResumeState { session_id, found, user_id, last_seq, resume_token, frames } => {
                hub.complete_migration(
                    session_id,
                    gateway::MigratedSession { found, user_id, last_seq, resume_token, frames },
                );
            }
            // SWIM 멤버십(D45): 상태머신에 합병 → 부수효과(송신/링 변형/dial)를 실행.
            m @ (M::SwimJoin { .. }
            | M::SwimPing { .. }
            | M::SwimAck { .. }
            | M::SwimPingReq { .. }
            | M::SwimGossip { .. }) => {
                let actions = swim.handle(src, &m, now);
                node::apply_swim_actions(&router, &on_member_up, now, actions).await;
            }
            other => match router.handle_inbound(Inbound { src, msg: other }).await {
                Ok(Some(delivery)) => gateway::deliver_local(&hub, &delivery).await,
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "inbound handle failed"),
            },
        }
    }
}

/// TLS 자재 로드: env 경로(TLS_CA/TLS_CERT/TLS_KEY)가 있으면 파일에서, 없으면 dev 임시 생성.
/// 멀티노드는 **공유 CA**가 필수이므로 운영/멀티노드 테스트는 파일 제공 권장.
fn load_tls(node_id: u64) -> Result<TlsMaterial, Box<dyn std::error::Error>> {
    match (std::env::var("TLS_CA"), std::env::var("TLS_CERT"), std::env::var("TLS_KEY")) {
        (Ok(ca), Ok(cert), Ok(key)) => Ok(TlsMaterial {
            ca_pem: std::fs::read_to_string(ca)?,
            cert_pem: std::fs::read_to_string(cert)?,
            key_pem: std::fs::read_to_string(key)?,
        }),
        _ => {
            tracing::warn!(node_id, "TLS_CA/CERT/KEY 미지정 — dev 임시 인증서 생성(멀티노드는 노드 간 신뢰 불가)");
            let mesh = transport::generate_mesh(&["127.0.0.1"])?;
            Ok(mesh.material(0))
        }
    }
}

/// "127.0.0.1:7002" → "127.0.0.1" (ServerName 검증용 호스트).
fn host_of(addr: &str) -> String {
    addr.rsplit_once(':').map(|(h, _)| h.to_string()).unwrap_or_else(|| addr.to_string())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// SWIM 파라미터를 env로 운영 노출 (H4, D45). 미설정 시 기본값. → (config, probe interval ms).
fn swim_config_from_env() -> (node::SwimConfig, u64) {
    let d = node::SwimConfig::default();
    let cfg = node::SwimConfig {
        ping_timeout_ms: env_parse("SWIM_PING_TIMEOUT_MS", d.ping_timeout_ms),
        probe_period_ms: env_parse("SWIM_PROBE_PERIOD_MS", d.probe_period_ms),
        suspicion_timeout_ms: env_parse("SWIM_SUSPICION_TIMEOUT_MS", d.suspicion_timeout_ms),
        indirect_k: env_parse("SWIM_INDIRECT_K", d.indirect_k),
        gossip_fanout: env_parse("SWIM_GOSSIP_FANOUT", d.gossip_fanout),
        dissemination_count: env_parse("SWIM_DISSEMINATION_COUNT", d.dissemination_count),
        max_piggyback: env_parse("SWIM_MAX_PIGGYBACK", d.max_piggyback),
        anti_entropy_ticks: env_parse("SWIM_ANTI_ENTROPY_TICKS", d.anti_entropy_ticks),
    };
    (cfg, env_parse("SWIM_PROBE_INTERVAL_MS", 500))
}

/// dev 인증서 생성: 공유 CA + 각 노드 cert(SAN=127.0.0.1) → `<out>/{ca,n<id>.cert,n<id>.key}.pem`.
fn gen_certs(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = args.first().ok_or("usage: gen-certs <out_dir> <id1> <id2> ...")?;
    let ids: Vec<&String> = args[1..].iter().collect();
    if ids.is_empty() {
        return Err("적어도 노드 id 1개 필요".into());
    }
    std::fs::create_dir_all(out)?;
    let hosts: Vec<&str> = ids.iter().map(|_| "127.0.0.1").collect();
    let mesh = transport::generate_mesh(&hosts)?;
    std::fs::write(format!("{out}/ca.pem"), &mesh.ca_pem)?;
    for (i, id) in ids.iter().enumerate() {
        let (cert, key) = &mesh.nodes[i];
        std::fs::write(format!("{out}/n{id}.cert.pem"), cert)?;
        std::fs::write(format!("{out}/n{id}.key.pem"), key)?;
    }
    println!("wrote CA + {} node cert(s) to {out}/", ids.len());
    Ok(())
}
