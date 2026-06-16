//! raw TCP + mTLS 전송 구현 (개념: tcp). in-process stub 교체 (D3/D4/D16).
//!
//! - 풀메시: 작은 id가 큰 id에게 dial(쌍당 1연결, D4 §6). 큰 id는 accept.
//! - 연결: TCP → mTLS 핸드셰이크(rustls) → HELLO 교환(헤더 src_node_id로 피어 식별) → 가동.
//! - 피어별 writer 태스크(mpsc 큐)로 송신 직렬화. reader는 프레임을 inbound 채널로.
//! - 끊기면 dial 측이 재연결(backoff). 송신 시 미연결 피어 → Unreachable.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::NodeMessage;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ServerConfig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::iface::{Inbound, NodeTransport, TransportError};
use crate::wire::{read_msg, write_msg};

type Peers = Arc<Mutex<HashMap<u64, tokio::sync::mpsc::Sender<NodeMessage>>>>;

/// raw TCP + mTLS 노드 전송.
#[derive(Clone)]
pub struct TcpTransport {
    local_node_id: u64,
    peers: Peers,
    /// dial을 이미 건 피어 — SWIM 동적 발견(D45) 시 중복 dial 루프 방지.
    dialing: Arc<Mutex<HashSet<u64>>>,
}

impl TcpTransport {
    pub fn new(local_node_id: u64) -> Self {
        Self {
            local_node_id,
            peers: Arc::new(Mutex::new(HashMap::new())),
            dialing: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 현재 핸드셰이크 완료(송신 가능)된 피어인가.
    pub fn is_connected(&self, peer_id: u64) -> bool {
        self.peers.lock().unwrap().contains_key(&peer_id)
    }

    /// 런타임 신규 피어 연결 (D45 — SWIM이 발견한 노드를 동적 dial). 이미 연결됐거나 dial 중이면 no-op(false).
    /// dial 정책(누가 누구에게)은 호출자(server)가 결정 — 보통 작은 id가 큰 id에게(D4 §6) + 합류 시 seed.
    pub fn connect_peer(
        &self,
        peer_id: u64,
        addr: String,
        host: String,
        client_cfg: Arc<ClientConfig>,
        inbound_tx: tokio::sync::mpsc::Sender<Inbound>,
    ) -> bool {
        {
            let mut dialing = self.dialing.lock().unwrap();
            if self.peers.lock().unwrap().contains_key(&peer_id) || dialing.contains(&peer_id) {
                return false;
            }
            dialing.insert(peer_id);
        }
        self.dial(peer_id, addr, host, client_cfg, inbound_tx);
        true
    }

    /// 수신 리스너 시작(accept 루프 spawn). 받은 메시지는 `inbound_tx`로.
    pub async fn listen(
        &self,
        addr: &str,
        server_cfg: Arc<ServerConfig>,
        inbound_tx: tokio::sync::mpsc::Sender<Inbound>,
    ) -> io::Result<std::net::SocketAddr> {
        let listener = TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;
        let acceptor = TlsAcceptor::from(server_cfg);
        let local_id = self.local_node_id;
        let peers = self.peers.clone();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _addr)) = listener.accept().await else { continue };
                let acceptor = acceptor.clone();
                let peers = peers.clone();
                let inbound = inbound_tx.clone();
                tokio::spawn(async move {
                    match acceptor.accept(tcp).await {
                        Ok(tls) => {
                            if let Err(e) = run_connection(tls, local_id, peers, inbound).await {
                                tracing::debug!("inbound connection ended: {e}");
                            }
                        }
                        Err(e) => tracing::warn!("tls accept failed: {e}"),
                    }
                });
            }
        });
        Ok(bound)
    }

    /// 피어 dial + 재연결 루프 spawn. `host`는 서버 인증서 SAN과 일치해야 함(검증).
    pub fn dial(
        &self,
        peer_id: u64,
        addr: String,
        host: String,
        client_cfg: Arc<ClientConfig>,
        inbound_tx: tokio::sync::mpsc::Sender<Inbound>,
    ) {
        let local_id = self.local_node_id;
        let peers = self.peers.clone();
        let connector = TlsConnector::from(client_cfg);
        tokio::spawn(async move {
            loop {
                match dial_once(&connector, &addr, &host, local_id, peers.clone(), inbound_tx.clone())
                    .await
                {
                    Ok(()) => tracing::info!(peer_id, "peer connection closed; reconnecting"),
                    Err(e) => tracing::debug!(peer_id, "dial failed: {e}; retrying"),
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }
}

impl NodeTransport for TcpTransport {
    fn local_node_id(&self) -> u64 {
        self.local_node_id
    }

    async fn send(&self, dest: u64, msg: NodeMessage) -> Result<(), TransportError> {
        let tx = {
            let guard = self.peers.lock().unwrap();
            guard.get(&dest).cloned()
        };
        match tx {
            Some(tx) => tx.send(msg).await.map_err(|_| TransportError::Unreachable(dest)),
            None => Err(TransportError::UnknownNode(dest)),
        }
    }
}

/// 한 번 dial → TLS connect → 연결 가동(reader가 끝날 때까지). 정상 종료 시 Ok.
async fn dial_once(
    connector: &TlsConnector,
    addr: &str,
    host: &str,
    local_id: u64,
    peers: Peers,
    inbound_tx: tokio::sync::mpsc::Sender<Inbound>,
) -> io::Result<()> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let tls = connector.connect(server_name, tcp).await?;
    run_connection(tls, local_id, peers, inbound_tx).await
}

/// 핸드셰이크(HELLO 교환) → 피어 등록 → writer 태스크 + reader(inline) 가동.
/// reader가 끝나면(연결 종료) 등록 해제 후 반환.
async fn run_connection<S>(
    stream: S,
    local_id: u64,
    peers: Peers,
    inbound_tx: tokio::sync::mpsc::Sender<Inbound>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(stream);

    // HELLO 교환: 양측 모두 먼저 보내고 그다음 읽음(데드락 방지).
    write_msg(&mut wr, local_id, &NodeMessage::Hello { capabilities: 0, epoch: 0 }).await?;
    let (peer_id, hello) = read_msg(&mut rd).await?;
    if !matches!(hello, NodeMessage::Hello { .. }) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "expected HELLO"));
    }
    tracing::info!(local_id, peer_id, "node connection established (mTLS)");

    // 피어별 writer 큐 등록.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<NodeMessage>(256);
    peers.lock().unwrap().insert(peer_id, tx);

    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if write_msg(&mut wr, local_id, &m).await.is_err() {
                break;
            }
        }
    });

    // reader inline: 연결이 끝날 때까지.
    loop {
        match read_msg(&mut rd).await {
            Ok((src, msg)) => {
                if inbound_tx.send(Inbound { src, msg }).await.is_err() {
                    break; // 상위 수신측 종료.
                }
            }
            Err(_) => break, // 연결 종료/오류.
        }
    }

    // 정리: 등록 해제(같은 peer가 재등록했으면 덮어쓰지 않도록 주의는 생략 — dial 측만 재연결).
    peers.lock().unwrap().remove(&peer_id);
    writer.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::{client_config, generate_mesh, server_config};

    fn install_crypto() {
        // ring provider 1회 설치 (테스트 다중 실행 안전).
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    /// 두 노드가 실제 mTLS(TCP) 위에서 메시지를 교환 (D3/D16 종단).
    #[tokio::test]
    async fn two_nodes_exchange_over_mtls() {
        install_crypto();
        // 공유 CA + 두 노드 cert(SAN=127.0.0.1).
        let mesh = generate_mesh(&["127.0.0.1", "127.0.0.1"]).expect("mesh certs");

        // 노드2(큰 id) listen, 노드1(작은 id) dial (D4 §6).
        let node2 = TcpTransport::new(2);
        let (in2_tx, mut in2_rx) = tokio::sync::mpsc::channel(16);
        let addr = node2
            .listen("127.0.0.1:0", server_config(&mesh.material(1)).unwrap(), in2_tx)
            .await
            .expect("listen");

        let node1 = TcpTransport::new(1);
        let (in1_tx, _in1_rx) = tokio::sync::mpsc::channel(16);
        node1.dial(2, addr.to_string(), "127.0.0.1".into(), client_config(&mesh.material(0)).unwrap(), in1_tx);

        // 핸드셰이크 완료까지 send 재시도(미연결이면 UnknownNode).
        let msg = NodeMessage::Subscribe { realm_id: 0x100, user_id: 0xA, node_id: 1 };
        let mut sent = false;
        for _ in 0..40 {
            if node1.send(2, msg.clone()).await.is_ok() {
                sent = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(sent, "dial/handshake가 2초 내 완료되지 않음");

        let got = tokio::time::timeout(Duration::from_secs(2), in2_rx.recv())
            .await
            .expect("배달 타임아웃")
            .expect("inbound 채널 종료");
        assert_eq!(got.src, 1, "프레임 헤더의 src_node_id로 피어 식별");
        assert_eq!(got.msg, msg);
    }

    /// 동적 dial(D45): 같은 피어로의 두 번째 connect_peer는 중복 dial을 막아 false 반환.
    #[tokio::test]
    async fn connect_peer_dedups() {
        install_crypto();
        let mesh = generate_mesh(&["127.0.0.1"]).unwrap();
        let t = TcpTransport::new(1);
        let (in_tx, _in_rx) = tokio::sync::mpsc::channel(8);
        let cfg = client_config(&mesh.material(0)).unwrap();
        assert!(!t.is_connected(2));
        let first = t.connect_peer(2, "127.0.0.1:65000".into(), "127.0.0.1".into(), cfg.clone(), in_tx.clone());
        let second = t.connect_peer(2, "127.0.0.1:65000".into(), "127.0.0.1".into(), cfg, in_tx);
        assert!(first, "첫 connect_peer는 dial 시작");
        assert!(!second, "이미 dial 중인 피어는 중복 dial 안 함");
    }

    /// mTLS: CA가 다른(신뢰 안 되는) 클라는 거부되어 메시지가 배달되지 않음.
    #[tokio::test]
    async fn untrusted_client_is_rejected() {
        install_crypto();
        let mesh = generate_mesh(&["127.0.0.1"]).unwrap(); // 서버용 CA
        let rogue = generate_mesh(&["127.0.0.1"]).unwrap(); // 다른 CA의 클라

        let server = TcpTransport::new(2);
        let (in_tx, mut in_rx) = tokio::sync::mpsc::channel(16);
        let addr = server
            .listen("127.0.0.1:0", server_config(&mesh.material(0)).unwrap(), in_tx)
            .await
            .unwrap();

        let client = TcpTransport::new(1);
        let (c_tx, _c_rx) = tokio::sync::mpsc::channel(16);
        client.dial(2, addr.to_string(), "127.0.0.1".into(), client_config(&rogue.material(0)).unwrap(), c_tx);

        // rogue는 핸드셰이크 실패 → 등록 안 됨 → send 계속 실패, 서버는 아무것도 못 받음.
        let msg = NodeMessage::Ping;
        for _ in 0..10 {
            let _ = client.send(2, msg.clone()).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(300), in_rx.recv()).await.is_err(),
            "신뢰 안 되는 CA의 클라 메시지가 배달되면 안 됨"
        );
    }
}
