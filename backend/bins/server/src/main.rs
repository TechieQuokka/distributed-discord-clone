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
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
    let router = Arc::new(Router::new(node_id, Arc::clone(&snowflakes), ring, transport.clone(), events_tx));
    let hub = Hub::new();

    // persist-then-fanout 드라이버 (D24).
    tokio::spawn(gateway::run_dispatch(events_rx, Arc::clone(&router), Arc::clone(&store), hub.clone()));

    // 멀티노드 전송 가동: listen + dial(작은 id가 큰 id에게) + 크로스노드 inbound 루프.
    if let Some(c) = &cluster {
        let tls = load_tls(node_id)?;
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(1024);
        transport
            .listen(&c.node.listen_addr, transport::server_config(&tls)?, inbound_tx.clone())
            .await?;
        for peer in c.peers_to_dial() {
            let host = host_of(&peer.addr);
            transport.dial(peer.id, peer.addr.clone(), host, transport::client_config(&tls)?, inbound_tx.clone());
        }
        // 수신: 원격 RealmSend→로컬 액터 / 원격 RealmFanout→로컬 세션 배달.
        let router2 = Arc::clone(&router);
        let hub2 = hub.clone();
        tokio::spawn(run_inbound(inbound_rx, router2, hub2));
        info!(node_id, listen = %c.node.listen_addr, peers = c.peers.len(), "node mesh active (mTLS)");
    } else {
        info!(node_id, "single-node mode (no cluster config)");
    }

    // REST(auth/guild/history) 라우터.
    let rest = rest_api::router(AppState::new(
        Arc::clone(&store),
        Arc::clone(&keys),
        Arc::clone(&snowflakes),
        Arc::clone(&clock),
    ));

    // Gateway(WS + 메시지 전송) 라우터.
    let gw = gateway::router(GatewayState {
        router: Arc::clone(&router),
        store: Arc::clone(&store),
        keys: Arc::clone(&keys),
        snowflakes: Arc::clone(&snowflakes),
        clock: Arc::clone(&clock),
        hub,
        local_node_id: node_id,
        heartbeat_interval_ms: 30_000,
    });

    let app = rest.merge(gw);
    let listener = tokio::net::TcpListener::bind(&rest_addr).await?;
    info!(addr = %rest_addr, node_id, worker_id, "listening: REST /auth /guilds /channels + WS /gateway");
    axum::serve(listener, app).await?;
    Ok(())
}

/// 원격 노드 메시지 처리 루프 (Phase 2 크로스노드 배달).
async fn run_inbound(
    mut rx: mpsc::Receiver<Inbound>,
    router: Arc<Router<TcpTransport>>,
    hub: Hub,
) {
    while let Some(inbound) = rx.recv().await {
        match router.handle_inbound(inbound).await {
            Ok(Some(delivery)) => gateway::deliver_local(&hub, &delivery).await,
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "inbound handle failed"),
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
