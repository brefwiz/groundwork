//! Environment- and file-driven configuration for [`crate::ServiceBootstrap`].

use std::path::Path;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Logging output format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Pretty, human-readable lines.
    #[default]
    Pretty,
    /// One JSON object per line — for log shippers.
    Json,
}

/// Backend store selection for [`RateLimitConfig`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RateLimitKind {
    /// No rate limiting.
    #[default]
    None,
    /// In-memory limiter (single-shard `HashMap`).
    Memory {
        /// Max requests per window.
        limit: u32,
        /// Window in seconds.
        window_secs: u64,
    },
    /// Postgres-backed limiter — requires `database_url` to be set.
    Postgres {
        /// Max requests per window.
        limit: u32,
        /// Window in seconds.
        window_secs: u64,
    },
    /// Redis-backed limiter.
    Redis {
        /// `redis://...` URL.
        url: String,
        /// Max requests per window.
        limit: u32,
        /// Window in seconds.
        window_secs: u64,
    },
}

/// Rate-limiting configuration for [`BootstrapConfig`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    /// Backend store + capacity settings.
    #[serde(flatten)]
    pub kind: RateLimitKind,

    /// Rate-limiting algorithm string (e.g. `"fixed_window"`, `"sliding_window"`).
    pub algorithm: Option<String>,

    /// Key-extraction strategy (e.g. `"ip"`, `"user_id"`, `"api_key"`).
    pub extractor: Option<String>,

    /// Trusted proxy CIDRs for the `"trusted_proxy"` extractor.
    pub trusted_proxy_cidrs: Vec<String>,

    /// Behaviour when the backing store is unavailable (`"fail_open"` or `"fail_closed"`).
    pub fail_mode: Option<String>,

    /// Lease tier (`"per_key"`, `"pooled"`, `"direct"`).
    pub lease_tier: Option<String>,

    /// Uniform ±`pct` random jitter applied to the `Retry-After` header.
    #[serde(default)]
    pub retry_after_jitter_pct: f64,

    /// Burst size override for token-bucket and GCRA algorithms.
    pub burst: Option<u64>,
}

/// Structured CORS configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorsConfig {
    /// Allowed origins (exact match). Use `["*"]` to allow any origin.
    pub allowed_origins: Vec<String>,
    /// Allowed methods.
    pub allowed_methods: Vec<String>,
    /// Allowed request headers.
    pub allowed_headers: Vec<String>,
    /// Headers exposed to the browser.
    pub expose_headers: Vec<String>,
    /// Whether to allow credentials.
    pub allow_credentials: bool,
    /// Preflight cache `max-age` in seconds.
    pub max_age_secs: Option<u64>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            allowed_methods: vec![
                "GET".into(),
                "POST".into(),
                "PUT".into(),
                "DELETE".into(),
                "PATCH".into(),
            ],
            allowed_headers: vec![
                "content-type".into(),
                "authorization".into(),
                "x-request-id".into(),
            ],
            expose_headers: Vec::new(),
            allow_credentials: false,
            max_age_secs: None,
        }
    }
}

/// Layered configuration consumed by [`crate::ServiceBootstrap::from_config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BootstrapConfig {
    /// Listener bind address. Default: `0.0.0.0:8080`.
    pub bind_addr: String,
    /// `tracing` env-filter directive. Default: `info`.
    pub log_level: String,
    /// Log output format. Default: `pretty`.
    pub log_format: LogFormat,
    /// Service version reported by the liveness endpoint.
    pub version: Option<String>,
    /// Health endpoint base path. Default: `/health`.
    pub health_path: String,
    /// OpenTelemetry collector endpoint. Honors `OTEL_EXPORTER_OTLP_ENDPOINT`.
    pub otel_endpoint: Option<String>,
    /// Postgres URL. Honors `DATABASE_URL`.
    pub database_url: Option<String>,
    /// Postgres pool max connections.
    pub pool_max_connections: Option<u32>,
    /// Rate-limit backend selection.
    pub rate_limit: RateLimitConfig,
    /// Maximum request body size in bytes. Default: 2 MiB.
    pub body_limit_bytes: usize,
    /// Graceful shutdown deadline in seconds. Default: 30.
    pub shutdown_timeout_secs: u64,
    /// CORS policy.
    pub cors: CorsConfig,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".into(),
            log_level: "info".into(),
            log_format: LogFormat::default(),
            version: None,
            health_path: "/health".into(),
            otel_endpoint: None,
            database_url: None,
            pool_max_connections: None,
            rate_limit: RateLimitConfig::default(),
            body_limit_bytes: 2 * 1024 * 1024,
            shutdown_timeout_secs: 30,
            cors: CorsConfig::default(),
        }
    }
}

impl BootstrapConfig {
    /// Load from environment variables only.
    pub fn from_env() -> Result<Self> {
        Self::figment(None::<&Path>).extract().map_err(map_err)
    }

    /// Load from a TOML file, with environment variables overriding any values present.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::figment(Some(path.as_ref()))
            .extract()
            .map_err(map_err)
    }

    fn figment(path: Option<&Path>) -> Figment {
        let mut fig = Figment::from(Serialized::defaults(Self::default()));
        if let Some(p) = path {
            fig = fig.merge(Toml::file(p));
        }
        fig.merge(Env::prefixed("GROUNDWORK_").split("__"))
            .merge(
                Env::raw()
                    .only(&["DATABASE_URL"])
                    .map(|_| "database_url".into()),
            )
            .merge(
                Env::raw()
                    .only(&["OTEL_EXPORTER_OTLP_ENDPOINT"])
                    .map(|_| "otel_endpoint".into()),
            )
    }

    /// Validate cross-field invariants.
    pub fn validate(self) -> Result<Self> {
        if matches!(self.rate_limit.kind, RateLimitKind::Postgres { .. })
            && self.database_url.is_none()
        {
            return Err(Error::Config(
                "rate_limit=postgres requires database_url to be set".into(),
            ));
        }
        Ok(self)
    }
}

fn map_err(e: figment::Error) -> Error {
    Error::Config(format!("config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let cfg = BootstrapConfig::default();
        assert_eq!(cfg.bind_addr, "0.0.0.0:8080");
        assert_eq!(cfg.health_path, "/health");
        assert_eq!(cfg.body_limit_bytes, 2 * 1024 * 1024);
        assert_eq!(cfg.shutdown_timeout_secs, 30);
        assert!(matches!(cfg.rate_limit.kind, RateLimitKind::None));
        assert!(matches!(cfg.log_format, LogFormat::Pretty));
    }

    #[test]
    fn validate_rejects_postgres_without_database_url() {
        let cfg = BootstrapConfig {
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Postgres {
                    limit: 1,
                    window_secs: 1,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_postgres_with_database_url() {
        let cfg = BootstrapConfig {
            database_url: Some("postgres://x".into()),
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Postgres {
                    limit: 1,
                    window_secs: 1,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }
}
