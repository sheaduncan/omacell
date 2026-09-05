//! `omacell ai setup`: detect loopback servers and sparse-patch user config.

use std::io::{Read, Write};
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
    /// HTTP protocol probe succeeded.
    pub reachable: bool,
}

/// Probe well-known loopback ports plus configured local providers.
#[must_use]
pub fn detect_local(config: &Config) -> Vec<DetectedProvider> {
    let mut found = Vec::new();
    for (name, host, port, probe_path, kind, endpoint) in [
        (
            "ollama",
            "127.0.0.1",
            11434,
            "/api/tags",
            "openai_compatible",
            "http://127.0.0.1:11434/v1",
        ),
        (
            "lmstudio",
            "127.0.0.1",
            1234,
            "/v1/models",
            "openai_compatible",
            "http://127.0.0.1:1234/v1",
        ),
        (
            "llamacpp",
            "127.0.0.1",
            8080,
            "/v1/models",
            "openai_compatible",
            "http://127.0.0.1:8080/v1",
        ),
        (
            "vllm",
            "127.0.0.1",
            8000,
            "/v1/models",
            "openai_compatible",
            "http://127.0.0.1:8000/v1",
        ),
    ] {
        let reachable = probe_http(host, port, probe_path, true);
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

fn endpoint_reachable(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let ip = if host.eq_ignore_ascii_case("localhost") {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        let Ok(ip) = host.parse::<std::net::IpAddr>() else {
            return false;
        };
        if !ip.is_loopback() {
            return false;
        }
        ip
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    probe_http(&ip.to_string(), port, path, false)
}

fn probe_http(host: &str, port: u16, path: &str, require_success: bool) -> bool {
    let Ok(ip) = host.parse() else {
        return false;
    };
    let Ok(mut stream) =
        TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_millis(80))
    else {
        return false;
    };
    let timeout = Some(Duration::from_millis(100));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = [0u8; 64];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    let line_end = response[..read]
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(read);
    let Some(line) = std::str::from_utf8(&response[..line_end])
        .ok()
        .map(str::trim_end)
    else {
        return false;
    };
    let mut fields = line.split_ascii_whitespace();
    let Some(version) = fields.next() else {
        return false;
    };
    let Some(status) = fields.next().and_then(|value| value.parse::<u16>().ok()) else {
        return false;
    };
    version.starts_with("HTTP/1.")
        && if require_success {
            (200..300).contains(&status)
        } else {
            (100..600).contains(&status)
        }
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
