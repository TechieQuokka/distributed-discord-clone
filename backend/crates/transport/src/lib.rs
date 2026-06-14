//! `transport` — 노드 전송 추상 + 구현 (D10/P3). 명세: `docs/protocol/node-wire.md`.
//!
//! 개념 모듈 분리 (CLAUDE.md R6):
//! - `iface` — `NodeTransport` trait + 에러 + `Inbound`
//! - `stub`  — in-process 구현(테스트/DST)
//! - `tls`   — mTLS 설정 + dev 인증서 생성 (D16)
//! - `wire`  — TLS 스트림 위 NodeMessage 프레이밍 입출력
//! - `tcp`   — raw TCP + mTLS 전송 구현 (D3/D16, Phase 2)

pub mod iface;
pub mod stub;
pub mod tcp;
pub mod tls;
pub mod wire;

pub use iface::{Inbound, NodeTransport, TransportError};
pub use stub::{InProcessTransport, Switchboard};
pub use tcp::TcpTransport;
pub use tls::{
    MeshCerts, TlsMaterial, client_config, generate_mesh, init_crypto, server_config,
};
