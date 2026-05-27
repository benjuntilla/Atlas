//! REST client for the Atlas control plane.
//!
//! The control plane is the backend the dashboard and CLI both talk to:
//! it provisions projects, manages API keys, exposes usage stats, and
//! streams logs. It will be built in a later phase.
//!
//! Until it exists, every method here has two implementations:
//!   - `real`: talks to whatever URL the CLI was configured with
//!   - `mock`: returns deterministic in-memory responses
//!
//! Selection is controlled by the `--mock` flag on the CLI (default on
//! while the control plane is not yet built). This lets us exercise the
//! whole CLI surface end to end without a running backend.

use crate::config::AtlasConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "http://localhost:8081/v1";

/// How the CLI talks to the control plane.
#[derive(Debug, Clone)]
pub enum Transport {
    /// Real HTTP against `base_url`. Used once the control plane ships.
    Http { base_url: String },
    /// In-memory mock. No network calls. Returns canned responses so the
    /// CLI is fully usable offline during platform development.
    Mock,
}

impl Transport {
    pub fn from_flags(mock: bool, base_url: Option<String>) -> Self {
        if mock {
            Transport::Mock
        } else {
            Transport::Http {
                base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            }
        }
    }
}

pub struct ApiClient {
    transport: Transport,
    api_key: String,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(transport: Transport, api_key: String) -> Self {
        Self {
            transport,
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// Convenience: build a client from a parsed config plus CLI flags.
    pub fn from_config(cfg: &AtlasConfig, mock: bool, base_url: Option<String>) -> Self {
        Self::new(
            Transport::from_flags(mock, base_url),
            cfg.project.api_key.clone(),
        )
    }

    // --- deploy ------------------------------------------------------------

    pub async fn deploy(&self, cfg: &AtlasConfig) -> Result<DeployResponse> {
        match &self.transport {
            Transport::Mock => Ok(DeployResponse::mock(cfg)),
            Transport::Http { base_url } => {
                let url = format!("{}/projects", base_url);
                let resp = self
                    .http
                    .post(url)
                    .bearer_auth(&self.api_key)
                    .json(&DeployRequest::from(cfg))
                    .send()
                    .await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("deploy failed ({}): {}", status, body));
                }
                Ok(resp.json::<DeployResponse>().await?)
            }
        }
    }

    // --- status ------------------------------------------------------------

    pub async fn status(&self, project_name: &str) -> Result<StatusResponse> {
        match &self.transport {
            Transport::Mock => Ok(StatusResponse::mock(project_name)),
            Transport::Http { base_url } => {
                let url = format!("{}/projects/{}/status", base_url, project_name);
                let resp = self.http.get(url).bearer_auth(&self.api_key).send().await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("status failed ({}): {}", status, body));
                }
                Ok(resp.json::<StatusResponse>().await?)
            }
        }
    }

    // --- logs --------------------------------------------------------------
    //
    // The real implementation will use SSE or chunked transfer to stream.
    // The mock returns a fixed-size vector so the CLI can render output
    // without holding a stream open.

    pub async fn logs(&self, project_name: &str, service: Option<&str>) -> Result<Vec<LogLine>> {
        match &self.transport {
            Transport::Mock => Ok(LogLine::mock_stream(service)),
            Transport::Http { base_url } => {
                let mut url = format!("{}/projects/{}/logs", base_url, project_name);
                if let Some(svc) = service {
                    url.push_str(&format!("?service={}", svc));
                }
                let resp = self.http.get(url).bearer_auth(&self.api_key).send().await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("logs failed ({}): {}", status, body));
                }
                Ok(resp.json::<Vec<LogLine>>().await?)
            }
        }
    }

    // --- keys --------------------------------------------------------------

    pub async fn list_keys(&self, project_name: &str) -> Result<Vec<ApiKey>> {
        match &self.transport {
            Transport::Mock => Ok(ApiKey::mock_list()),
            Transport::Http { base_url } => {
                let url = format!("{}/projects/{}/keys", base_url, project_name);
                let resp = self.http.get(url).bearer_auth(&self.api_key).send().await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("list_keys failed ({}): {}", status, body));
                }
                Ok(resp.json::<Vec<ApiKey>>().await?)
            }
        }
    }

    pub async fn create_key(
        &self,
        project_name: &str,
        name: &str,
        expiry: KeyExpiry,
    ) -> Result<ApiKey> {
        match &self.transport {
            Transport::Mock => Ok(ApiKey::mock_create(name, expiry)),
            Transport::Http { base_url } => {
                let url = format!("{}/projects/{}/keys", base_url, project_name);
                let body = CreateKeyRequest {
                    name: name.to_string(),
                    expiry,
                };
                let resp = self
                    .http
                    .post(url)
                    .bearer_auth(&self.api_key)
                    .json(&body)
                    .send()
                    .await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("create_key failed ({}): {}", status, body));
                }
                Ok(resp.json::<ApiKey>().await?)
            }
        }
    }

    pub async fn revoke_key(&self, project_name: &str, key_prefix: &str) -> Result<()> {
        match &self.transport {
            Transport::Mock => Ok(()),
            Transport::Http { base_url } => {
                let url = format!("{}/projects/{}/keys/{}", base_url, project_name, key_prefix);
                let resp = self
                    .http
                    .delete(url)
                    .bearer_auth(&self.api_key)
                    .send()
                    .await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("revoke_key failed ({}): {}", status, body));
                }
                Ok(())
            }
        }
    }
}

// --- request / response types ---------------------------------------------

#[derive(Debug, Serialize)]
pub struct DeployRequest<'a> {
    pub name: &'a str,
    pub region: &'a str,
    pub environment: &'a str,
    pub services_enabled: Vec<&'static str>,
}

impl<'a> From<&'a AtlasConfig> for DeployRequest<'a> {
    fn from(cfg: &'a AtlasConfig) -> Self {
        let mut services = Vec::new();
        if cfg.services.auth {
            services.push("auth");
        }
        if cfg.services.geo {
            services.push("geo");
        }
        if cfg.services.payments {
            services.push("payments");
        }
        if cfg.services.events {
            services.push("events");
        }
        DeployRequest {
            name: &cfg.project.name,
            region: &cfg.project.region,
            environment: cfg.project.environment.as_str(),
            services_enabled: services,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployResponse {
    pub project_name: String,
    pub region: String,
    pub provisioned: Vec<ProvisionedService>,
    pub endpoint: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProvisionedService {
    pub service: String,
    pub status: String, // "ok" | "skipped" | "failed"
    pub detail: Option<String>,
}

impl DeployResponse {
    fn mock(cfg: &AtlasConfig) -> Self {
        let mut provisioned = Vec::new();
        if cfg.services.auth {
            provisioned.push(ProvisionedService {
                service: "auth".into(),
                status: "ok".into(),
                detail: None,
            });
        }
        if cfg.services.geo {
            provisioned.push(ProvisionedService {
                service: "geo".into(),
                status: "ok".into(),
                detail: None,
            });
        }
        if cfg.services.payments {
            provisioned.push(ProvisionedService {
                service: "payments".into(),
                status: "ok".into(),
                detail: None,
            });
        }
        if cfg.services.events {
            let count = cfg.events.as_ref().map(|e| e.topics.len()).unwrap_or(0);
            provisioned.push(ProvisionedService {
                service: "events".into(),
                status: "ok".into(),
                detail: Some(format!("{} topic(s)", count)),
            });
        }
        DeployResponse {
            project_name: cfg.project.name.clone(),
            region: cfg.project.region.clone(),
            provisioned,
            endpoint: format!("https://api.atlas.dev/v1/{}", cfg.project.name),
            elapsed_ms: 3_200,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub project_name: String,
    pub services: Vec<ServiceStatus>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub healthy: bool,
    pub p95_latency_ms: u32,
    pub requests_24h: u64,
    pub error_rate: f64,
}

impl StatusResponse {
    fn mock(project_name: &str) -> Self {
        StatusResponse {
            project_name: project_name.to_string(),
            services: vec![
                ServiceStatus {
                    name: "auth".into(),
                    healthy: true,
                    p95_latency_ms: 28,
                    requests_24h: 14_203,
                    error_rate: 0.0002,
                },
                ServiceStatus {
                    name: "geo".into(),
                    healthy: true,
                    p95_latency_ms: 41,
                    requests_24h: 88_417,
                    error_rate: 0.0009,
                },
                ServiceStatus {
                    name: "payments".into(),
                    healthy: true,
                    p95_latency_ms: 63,
                    requests_24h: 2_104,
                    error_rate: 0.0,
                },
                ServiceStatus {
                    name: "events".into(),
                    healthy: true,
                    p95_latency_ms: 12,
                    requests_24h: 412_338,
                    error_rate: 0.0,
                },
            ],
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogLine {
    pub timestamp: String,
    pub service: String,
    pub level: String, // "info" | "warn" | "error"
    pub message: String,
}

impl LogLine {
    fn mock_stream(service: Option<&str>) -> Vec<LogLine> {
        let all = vec![
            LogLine {
                timestamp: "2026-05-26T15:42:11Z".into(),
                service: "auth".into(),
                level: "info".into(),
                message: "issued token for user_a1b2c3".into(),
            },
            LogLine {
                timestamp: "2026-05-26T15:42:14Z".into(),
                service: "geo".into(),
                level: "info".into(),
                message: "nearby query: 12 results in 38ms".into(),
            },
            LogLine {
                timestamp: "2026-05-26T15:42:17Z".into(),
                service: "geo".into(),
                level: "warn".into(),
                message: "elo recompute deferred — db lock contention".into(),
            },
            LogLine {
                timestamp: "2026-05-26T15:42:19Z".into(),
                service: "payments".into(),
                level: "info".into(),
                message: "settled tx 0d7f… for ride 9c2b…".into(),
            },
            LogLine {
                timestamp: "2026-05-26T15:42:22Z".into(),
                service: "events".into(),
                level: "info".into(),
                message: "produced fare.events: 142 msgs in last minute".into(),
            },
            LogLine {
                timestamp: "2026-05-26T15:42:25Z".into(),
                service: "auth".into(),
                level: "error".into(),
                message: "token validation failed: signature mismatch".into(),
            },
        ];
        match service {
            None => all,
            Some(s) => all.into_iter().filter(|l| l.service == s).collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKey {
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub status: String, // "active" | "revoked"
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum KeyExpiry {
    Never,
    Days30,
    Days90,
    Days365,
}

impl std::str::FromStr for KeyExpiry {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "never" => Ok(KeyExpiry::Never),
            "30d" | "30" | "days30" => Ok(KeyExpiry::Days30),
            "90d" | "90" | "days90" => Ok(KeyExpiry::Days90),
            "1y" | "365d" | "365" | "days365" => Ok(KeyExpiry::Days365),
            _ => Err(anyhow!(
                "invalid expiry '{}': expected one of never | 30d | 90d | 1y",
                s
            )),
        }
    }
}

#[derive(Debug, Serialize)]
struct CreateKeyRequest {
    name: String,
    expiry: KeyExpiry,
}

impl ApiKey {
    fn mock_list() -> Vec<ApiKey> {
        vec![
            ApiKey {
                name: "primary".into(),
                prefix: "atl_live_abcd".into(),
                created_at: "2026-04-12T09:14:33Z".into(),
                last_used_at: Some("2026-05-26T15:41:02Z".into()),
                status: "active".into(),
            },
            ApiKey {
                name: "ci".into(),
                prefix: "atl_live_ef01".into(),
                created_at: "2026-04-20T11:02:18Z".into(),
                last_used_at: Some("2026-05-26T14:28:55Z".into()),
                status: "active".into(),
            },
            ApiKey {
                name: "old-dev-key".into(),
                prefix: "atl_dev_2345".into(),
                created_at: "2026-01-05T08:00:00Z".into(),
                last_used_at: None,
                status: "revoked".into(),
            },
        ]
    }

    fn mock_create(name: &str, _expiry: KeyExpiry) -> ApiKey {
        ApiKey {
            name: name.to_string(),
            prefix: "atl_live_6789".into(),
            created_at: "2026-05-26T15:42:30Z".into(),
            last_used_at: None,
            status: "active".into(),
        }
    }
}
