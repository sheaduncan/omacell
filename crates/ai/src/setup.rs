//! `omacell ai setup`: detect loopback servers and sparse-patch user config.

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use omacell_conf::schema::Config;
use serde::Serialize;

use crate::error::{AiError, codes};
use crate::provider::endpoint_is_loopback;

/// One detected (or configured) local endpoint.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DetectedProvider {
    /// Config key (`ollama`).
    pub name: String,
    /// Kind.
    pub kind: String,
    /// Endpoint that will be used.
    pub endpoint: String,
    /// TCP probe succeeded.
    pub reachable: bool,
}

/// Probe well-known loopback ports plus configured local providers.
#[must_use]
pub fn detect_local(config: &Config) -> Vec<DetectedProvider> {
    let mut found = Vec::new();
    for (name, host, port, kind, endpoint) in [
        (
            "ollama",
            "127.0.0.1",
            11434,
            "openai_compatible",
            "http://127.0.0.1:11434/v1",
        ),
        (
            "lmstudio",
            "127.0.0.1",
            1234,
            "openai_compatible",
            "http://127.0.0.1:1234/v1",
        ),
        (
            "llamacpp",
            "127.0.0.1",
            8080,
            "openai_compatible",
            "http://127.0.0.1:8080/v1",
        ),
        (
            "vllm",
            "127.0.0.1",
            8000,
            "openai_compatible",
            "http://127.0.0.1:8000/v1",
        ),
    ] {
        let reachable = probe(host, port);
        if reachable {
            found.push(DetectedProvider {
                name: name.into(),
                kind: kind.into(),
                endpoint: endpoint.into(),
                reachable: true,
            });
        }
    }
    for (name, spec) in &config.ai.providers {
        if spec.local || endpoint_is_loopback(&spec.endpoint) {
            if found.iter().any(|d| d.name == *name) {
                continue;
            }
            let reachable = endpoint_reachable(&spec.endpoint);
            if reachable {
                found.push(DetectedProvider {
                    name: name.clone(),
                    kind: spec.kind.clone(),
                    endpoint: spec.endpoint.clone(),
                    reachable: true,
                });
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn probe(host: &str, port: u16) -> bool {
    let addr = SocketAddr::new(
        host.parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        port,
    );
    TcpStream::connect_timeout(&addr, Duration::from_millis(80)).is_ok()
}

fn endpoint_reachable(endpoint: &str) -> bool {
    let rest = endpoint
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(endpoint);
    let hostport = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(80u16)),
        None => (hostport, 80),
    };
    let host = host.trim_matches(|c| c == '[' || c == ']');
    probe(host, port)
}

/// Keys written by setup (never secrets).
#[derive(Clone, Debug)]
pub struct SetupPatch {
    /// `ai.enabled`.
    pub enabled: bool,
    /// Detected providers to upsert.
    pub providers: Vec<DetectedProvider>,
}

impl SetupPatch {
    /// From a detection pass.
    #[must_use]
    pub fn from_detected(detected: Vec<DetectedProvider>) -> Self {
        Self {
            // Never disable a previously enabled cloud setup when no local
            // server is found; only turn AI on when a loopback server exists.
            enabled: !detected.is_empty(),
            providers: detected,
        }
    }
}

/// Sparse-patch `config.toml`. `toml_edit` lives in conf.
pub fn apply_setup_patch(path: &Path, patch: &SetupPatch) -> Result<(), AiError> {
    let providers: Vec<(&str, &str, &str)> = patch
        .providers
        .iter()
        .map(|p| (p.name.as_str(), p.kind.as_str(), p.endpoint.as_str()))
        .collect();
    omacell_conf::edit::patch_ai_setup(path, patch.enabled, &providers)
        .map_err(|err| AiError::new(codes::SETUP, err.to_string()))
}
