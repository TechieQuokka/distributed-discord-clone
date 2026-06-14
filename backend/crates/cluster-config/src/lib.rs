//! `cluster-config` — 정적 클러스터 설정 (D5/D29). 노드 목록 + worker-id.
//! TOML 파일에서 로드. umbrella 워크스페이스 없이 독립 관리 (R7).

use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("toml parse error: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// 이 노드 + 피어들 (정적 발견, D5).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClusterConfig {
    pub node: NodeConfig,
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NodeConfig {
    pub id: u64,
    /// Snowflake worker id (D29), 0..=1023.
    pub worker_id: u16,
    /// 노드 간 raw TCP 리슨 주소 (Phase 2).
    pub listen_addr: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PeerConfig {
    pub id: u64,
    pub addr: String,
}

impl ClusterConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let cfg: ClusterConfig = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, ConfigError> {
        let s = std::fs::read_to_string(path).map_err(|e| ConfigError::Invalid(e.to_string()))?;
        Self::from_toml_str(&s)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.node.worker_id > 1023 {
            return Err(ConfigError::Invalid(format!(
                "worker_id {} out of range (0..=1023)",
                self.node.worker_id
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.peers {
            if p.id == self.node.id {
                return Err(ConfigError::Invalid("peer list must not contain self".into()));
            }
            if !seen.insert(p.id) {
                return Err(ConfigError::Invalid(format!("duplicate peer id {}", p.id)));
            }
        }
        Ok(())
    }

    /// 풀메시 연결 시 "내가 dial할 피어" = id가 나보다 큰 피어 (D4: 쌍당 1연결).
    pub fn peers_to_dial(&self) -> impl Iterator<Item = &PeerConfig> {
        self.peers.iter().filter(move |p| p.id > self.node.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [node]
        id = 1
        worker_id = 1
        listen_addr = "127.0.0.1:7001"

        [[peers]]
        id = 2
        addr = "127.0.0.1:7002"

        [[peers]]
        id = 3
        addr = "127.0.0.1:7003"
    "#;

    #[test]
    fn parses_and_selects_dial_targets() {
        let cfg = ClusterConfig::from_toml_str(SAMPLE).unwrap();
        assert_eq!(cfg.node.id, 1);
        assert_eq!(cfg.node.worker_id, 1);
        assert_eq!(cfg.peers.len(), 2);
        // 나(1)보다 큰 피어 = 2,3 → 둘 다 dial
        assert_eq!(cfg.peers_to_dial().count(), 2);
    }

    #[test]
    fn rejects_self_in_peers() {
        let bad = r#"
            [node]
            id = 1
            worker_id = 1
            listen_addr = "x"
            [[peers]]
            id = 1
            addr = "y"
        "#;
        assert!(ClusterConfig::from_toml_str(bad).is_err());
    }

    #[test]
    fn rejects_out_of_range_worker_id() {
        let bad = r#"
            [node]
            id = 1
            worker_id = 2000
            listen_addr = "x"
        "#;
        assert!(ClusterConfig::from_toml_str(bad).is_err());
    }
}
