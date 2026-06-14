//! mTLS 설정 (개념: tls). 노드↔노드 채널 인증·암호화 (D16, P6: rustls).
//!
//! 공유 self-signed CA + 노드별 cert. 서버측은 클라 인증서를 CA로 검증(mTLS),
//! 클라측은 서버 인증서를 CA로 검증 + 자기 인증서 제시 → 상호 인증.
//! 개발/테스트용 인증서 생성기(rcgen)도 제공(운영은 PEM 파일 로드).

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("pem parse: {0}")]
    Pem(String),
    #[error("rustls: {0}")]
    Rustls(String),
    #[error("cert gen: {0}")]
    Gen(String),
    #[error("no private key in pem")]
    NoKey,
}

/// 프로세스당 1회 rustls 암호화 제공자(ring) 설치. 멱등(이미 설치돼 있으면 무시).
/// rustls 사용(서버/클라 config 생성·핸드셰이크) 전에 호출.
pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 한 노드의 TLS 자재 (PEM). 운영은 파일에서 로드, 개발은 [`generate_mesh`]로 생성.
#[derive(Clone, Debug)]
pub struct TlsMaterial {
    pub ca_pem: String,
    pub cert_pem: String,
    pub key_pem: String,
}

fn parse_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    rustls_pemfile::certs(&mut pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Pem(e.to_string()))
}

fn parse_key(pem: &str) -> Result<PrivateKeyDer<'static>, TlsError> {
    rustls_pemfile::private_key(&mut pem.as_bytes())
        .map_err(|e| TlsError::Pem(e.to_string()))?
        .ok_or(TlsError::NoKey)
}

fn root_store(ca_pem: &str) -> Result<RootCertStore, TlsError> {
    let mut roots = RootCertStore::empty();
    for cert in parse_certs(ca_pem)? {
        roots.add(cert).map_err(|e| TlsError::Rustls(e.to_string()))?;
    }
    Ok(roots)
}

/// 서버(수신)측 mTLS 설정: 접속한 피어의 인증서를 CA로 검증.
pub fn server_config(m: &TlsMaterial) -> Result<Arc<ServerConfig>, TlsError> {
    let roots = Arc::new(root_store(&m.ca_pem)?);
    let verifier = WebPkiClientVerifier::builder(roots)
        .build()
        .map_err(|e| TlsError::Rustls(e.to_string()))?;
    let cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(parse_certs(&m.cert_pem)?, parse_key(&m.key_pem)?)
        .map_err(|e| TlsError::Rustls(e.to_string()))?;
    Ok(Arc::new(cfg))
}

/// 클라(발신)측 mTLS 설정: 서버 인증서를 CA로 검증 + 자기 인증서 제시.
pub fn client_config(m: &TlsMaterial) -> Result<Arc<ClientConfig>, TlsError> {
    let roots = root_store(&m.ca_pem)?;
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(parse_certs(&m.cert_pem)?, parse_key(&m.key_pem)?)
        .map_err(|e| TlsError::Rustls(e.to_string()))?;
    Ok(Arc::new(cfg))
}

/// 메시 전체의 개발/테스트 인증서: 공유 CA + 호스트별 노드 cert.
pub struct MeshCerts {
    pub ca_pem: String,
    /// hosts 순서대로 (cert_pem, key_pem).
    pub nodes: Vec<(String, String)>,
}

impl MeshCerts {
    /// `i`번째 노드의 TlsMaterial.
    pub fn material(&self, i: usize) -> TlsMaterial {
        let (cert_pem, key_pem) = self.nodes[i].clone();
        TlsMaterial { ca_pem: self.ca_pem.clone(), cert_pem, key_pem }
    }
}

/// 공유 CA로 각 host(SAN)에 노드 인증서를 발급 (로컬 메시 dev/test, D16).
pub fn generate_mesh(hosts: &[&str]) -> Result<MeshCerts, TlsError> {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};

    let ca_key = KeyPair::generate().map_err(|e| TlsError::Gen(e.to_string()))?;
    let mut ca_params = CertificateParams::new(Vec::new()).map_err(|e| TlsError::Gen(e.to_string()))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).map_err(|e| TlsError::Gen(e.to_string()))?;
    let ca_pem = ca_cert.pem();
    // CA를 발급자로 — 이후 노드 cert에 서명.
    let issuer = Issuer::new(ca_params, ca_key);

    let mut nodes = Vec::with_capacity(hosts.len());
    for host in hosts {
        let key = KeyPair::generate().map_err(|e| TlsError::Gen(e.to_string()))?;
        let params = CertificateParams::new(vec![host.to_string()])
            .map_err(|e| TlsError::Gen(e.to_string()))?;
        let cert = params
            .signed_by(&key, &issuer)
            .map_err(|e| TlsError::Gen(e.to_string()))?;
        nodes.push((cert.pem(), key.serialize_pem()));
    }
    Ok(MeshCerts { ca_pem, nodes })
}
