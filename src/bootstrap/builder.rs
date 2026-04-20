//! `ServiceBootstrap` builder — chains `with_*` methods and calls `serve()`.

use std::sync::Arc;

use axum::Router;
use tower_http::cors::CorsLayer;

use crate::bootstrap::ctx::BootstrapCtx;
use crate::config::{BootstrapConfig, CorsConfig, RateLimitKind};
use crate::error::{Error, Result};
use crate::ports::health::{HealthProbe, ReadinessCheckFn, probe_to_check_fn};

#[cfg(feature = "ratelimit")]
use crate::adapters::security::rate_limit::{RateLimitBackend, RateLimitExtractor};

// ─── Internal types ───────────────────────────────────────────────────────────

pub(crate) type RouterBuilder = Box<dyn FnOnce(&BootstrapCtx) -> Router + Send>;

/// Async drain callback registered via [`ServiceBootstrap::with_shutdown_hook`].
pub type ShutdownHookFn =
    Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

pub(crate) struct ShutdownHook {
    pub(crate) name: String,
    pub(crate) hook: ShutdownHookFn,
    pub(crate) timeout: std::time::Duration,
}

// ─── ServiceBootstrap ────────────────────────────────────────────────────────

/// Builder for a microservice runtime.
///
/// ```rust,no_run
/// use groundwork::{ServiceBootstrap, BootstrapCtx, Result};
/// use axum::{Router, routing::get};
///
/// # #[tokio::main] async fn main() -> Result<()> {
/// ServiceBootstrap::new("my-service")
///     .with_telemetry()
///     .with_router(|_ctx: &BootstrapCtx| Router::new().route("/health", get(|| async { "ok" })))
///     .serve("0.0.0.0:8080")
///     .await
/// # }
/// ```
#[must_use = "ServiceBootstrap does nothing until you call .serve()"]
pub struct ServiceBootstrap {
    pub(crate) service_name: Arc<str>,

    #[cfg(feature = "telemetry")]
    pub(crate) telemetry: bool,

    #[cfg(feature = "prometheus-metrics")]
    pub(crate) prometheus_metrics: bool,
    #[cfg(feature = "prometheus-metrics")]
    pub(crate) prometheus_path: String,

    #[cfg(feature = "database")]
    pub(crate) database_url: Option<String>,
    #[cfg(feature = "database")]
    pub(crate) migrator: Option<sqlx::migrate::Migrator>,

    #[cfg(feature = "ratelimit")]
    pub(crate) rate_limit: Option<RateLimitBackend>,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_extractor: RateLimitExtractor,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_fail_open: bool,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_burst: Option<u64>,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_algorithm: Option<String>,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_retry_after_jitter_pct: f64,

    pub(crate) cors: Option<CorsLayer>,
    pub(crate) router_builder: Option<RouterBuilder>,
    pub(crate) version: String,
    pub(crate) health_path: String,
    pub(crate) body_limit_bytes: usize,
    pub(crate) shutdown_timeout: std::time::Duration,
    pub(crate) shutdown_hooks: Vec<ShutdownHook>,
    pub(crate) readiness_checks: Vec<(String, ReadinessCheckFn)>,
    pub(crate) bind_addr: Option<String>,

    #[cfg(feature = "openapi")]
    pub(crate) openapi: Option<utoipa::openapi::OpenApi>,
    #[cfg(feature = "openapi")]
    pub(crate) openapi_spec_path: String,
    #[cfg(feature = "openapi")]
    pub(crate) openapi_ui_path: String,

    #[cfg(feature = "auth")]
    pub(crate) auth_layer: Option<crate::auth::AuthLayer>,
}

impl ServiceBootstrap {
    /// Start a new bootstrap for a service.
    pub fn new(service_name: impl Into<Arc<str>>) -> Self {
        #[cfg(feature = "dotenv")]
        let _ = dotenvy::dotenv();

        Self {
            service_name: service_name.into(),
            #[cfg(feature = "telemetry")]
            telemetry: false,
            #[cfg(feature = "prometheus-metrics")]
            prometheus_metrics: false,
            #[cfg(feature = "prometheus-metrics")]
            prometheus_path: "/metrics".to_string(),
            #[cfg(feature = "database")]
            database_url: None,
            #[cfg(feature = "database")]
            migrator: None,
            #[cfg(feature = "ratelimit")]
            rate_limit: None,
            #[cfg(feature = "ratelimit")]
            ratelimit_extractor: RateLimitExtractor::Ip,
            #[cfg(feature = "ratelimit")]
            ratelimit_fail_open: true,
            #[cfg(feature = "ratelimit")]
            ratelimit_burst: None,
            #[cfg(feature = "ratelimit")]
            ratelimit_algorithm: None,
            #[cfg(feature = "ratelimit")]
            ratelimit_retry_after_jitter_pct: 0.0,
            cors: None,
            router_builder: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            health_path: "/health".to_string(),
            body_limit_bytes: 2 * 1024 * 1024,
            shutdown_timeout: std::time::Duration::from_secs(30),
            shutdown_hooks: Vec::new(),
            readiness_checks: Vec::new(),
            bind_addr: None,
            #[cfg(feature = "openapi")]
            openapi: None,
            #[cfg(feature = "openapi")]
            openapi_spec_path: "/openapi.json".into(),
            #[cfg(feature = "openapi")]
            openapi_ui_path: "/docs".into(),
            #[cfg(feature = "auth")]
            auth_layer: None,
        }
    }

    /// Build from a [`BootstrapConfig`].
    pub fn from_config(service_name: impl Into<Arc<str>>, cfg: BootstrapConfig) -> Result<Self> {
        let cfg = cfg.validate()?;
        let mut b = Self::new(service_name)
            .with_health_path(cfg.health_path)
            .with_body_limit(cfg.body_limit_bytes)
            .with_shutdown_timeout(std::time::Duration::from_secs(cfg.shutdown_timeout_secs));
        if let Some(v) = cfg.version {
            b = b.with_version(v);
        }
        b.bind_addr = Some(cfg.bind_addr);

        if cfg.cors != CorsConfig::default() {
            b = b.with_cors_config(cfg.cors)?;
        }

        #[cfg(feature = "telemetry")]
        if cfg.otel_endpoint.is_some() {
            b = b.with_telemetry();
        }

        #[cfg(feature = "database")]
        if let Some(url) = cfg.database_url {
            b = b.with_database(url);
        }

        #[cfg(feature = "ratelimit")]
        {
            let rl_cfg = cfg.rate_limit;
            match rl_cfg.kind {
                RateLimitKind::None => {}
                #[cfg(feature = "ratelimit-memory")]
                RateLimitKind::Memory { limit, window_secs } => {
                    b = b.with_rate_limit(RateLimitBackend::Memory { limit, window_secs });
                }
                #[cfg(not(feature = "ratelimit-memory"))]
                RateLimitKind::Memory { .. } => {
                    return Err(Error::Config(
                        "rate_limit=memory requires feature ratelimit-memory".into(),
                    ));
                }
                #[cfg(feature = "ratelimit-postgres")]
                RateLimitKind::Postgres { limit, window_secs } => {
                    b = b.with_rate_limit(RateLimitBackend::Postgres { limit, window_secs });
                }
                #[cfg(not(feature = "ratelimit-postgres"))]
                RateLimitKind::Postgres { .. } => {
                    return Err(Error::Config(
                        "rate_limit=postgres requires feature ratelimit-postgres".into(),
                    ));
                }
                #[cfg(feature = "ratelimit-redis")]
                RateLimitKind::Redis {
                    url,
                    limit,
                    window_secs,
                } => {
                    b = b.with_rate_limit(RateLimitBackend::redis_from_url(
                        &url,
                        limit,
                        window_secs,
                    )?);
                }
                #[cfg(not(feature = "ratelimit-redis"))]
                RateLimitKind::Redis { .. } => {
                    return Err(Error::Config(
                        "rate_limit=redis requires feature ratelimit-redis".into(),
                    ));
                }
            }

            if let Some(alg_str) = rl_cfg.algorithm {
                b = b.with_rate_limit_algorithm(alg_str);
            }

            if let Some(burst) = rl_cfg.burst {
                b = b.with_rate_limit_burst(burst);
            }

            b = b.with_rate_limit_retry_after_jitter_pct(rl_cfg.retry_after_jitter_pct);
        }

        Ok(b)
    }

    /// Run using the bind address loaded from [`BootstrapConfig`].
    pub async fn run(self) -> Result<()> {
        let addr = self.bind_addr.clone().ok_or_else(|| {
            Error::Config("run() requires from_config(...) to set bind_addr".into())
        })?;
        self.serve(addr).await
    }

    // ── Core settings ──────────────────────────────────────────────────────────

    /// Override the maximum request body size in bytes. Defaults to 2 MiB.
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit_bytes = bytes;
        self
    }

    /// Hard deadline on graceful shutdown drain. Defaults to 30 seconds.
    pub fn with_shutdown_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Register an async drain callback that runs after the HTTP server stops.
    pub fn with_shutdown_hook<F, Fut>(
        mut self,
        name: impl Into<String>,
        timeout: std::time::Duration,
        hook: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.shutdown_hooks.push(ShutdownHook {
            name: name.into(),
            hook: Arc::new(move || Box::pin(hook())),
            timeout,
        });
        self
    }

    /// Register a readiness check.
    pub fn with_readiness_check<F, Fut>(mut self, name: impl Into<String>, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = api_bones::health::HealthCheck> + Send + 'static,
    {
        let f: ReadinessCheckFn = Arc::new(move || Box::pin(check()));
        self.readiness_checks.push((name.into(), f));
        self
    }

    /// Register a typed [`HealthProbe`] as a readiness check.
    pub fn with_health_probe(mut self, probe: impl HealthProbe + 'static) -> Self {
        self.readiness_checks.push(probe_to_check_fn(probe));
        self
    }

    /// Override the version reported by the liveness endpoint.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Override the base path for health endpoints. Defaults to `/health`.
    pub fn with_health_path(mut self, path: impl Into<String>) -> Self {
        self.health_path = path.into();
        self
    }

    // ── Telemetry ──────────────────────────────────────────────────────────────

    /// Enable basic tracing via `tracing_subscriber`.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry(mut self) -> Self {
        self.telemetry = true;
        self
    }

    /// Mount a Prometheus scrape endpoint at `GET /metrics`.
    #[cfg(feature = "prometheus-metrics")]
    pub fn with_prometheus_metrics(mut self) -> Self {
        self.prometheus_metrics = true;
        self
    }

    /// Override the default `/metrics` mount path.
    #[cfg(feature = "prometheus-metrics")]
    pub fn with_prometheus_path(mut self, path: impl Into<String>) -> Self {
        self.prometheus_path = path.into();
        self
    }

    // ── Database ───────────────────────────────────────────────────────────────

    /// Connect to a Postgres database and build a `sqlx::PgPool`.
    #[cfg(feature = "database")]
    pub fn with_database(mut self, url: impl Into<String>) -> Self {
        self.database_url = Some(url.into());
        self
    }

    /// Run sqlx migrations at startup.
    #[cfg(feature = "database")]
    pub fn with_migrations(mut self, migrator: sqlx::migrate::Migrator) -> Self {
        self.migrator = Some(migrator);
        self
    }

    // ── Rate limiting ──────────────────────────────────────────────────────────

    /// Enable rate limiting with the given store backend.
    ///
    /// Note: the middleware is not yet wired into the tower stack.
    /// This stores the configuration for future use. See the `ratelimit` module.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit(mut self, config: RateLimitBackend) -> Self {
        // TODO: wire actual rate limiting middleware
        self.rate_limit = Some(config);
        self
    }

    /// Override the key extractor used by the rate limiter.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_extractor(mut self, extractor: RateLimitExtractor) -> Self {
        self.ratelimit_extractor = extractor;
        self
    }

    /// Override the rate-limit algorithm (e.g. `"sliding_window"`, `"gcra"`).
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.ratelimit_algorithm = Some(algorithm.into());
        self
    }

    /// Switch to fail-closed mode.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_fail_closed(mut self) -> Self {
        self.ratelimit_fail_open = false;
        self
    }

    /// Apply uniform ±`pct` jitter to `Retry-After` on HTTP 429 responses.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_retry_after_jitter_pct(mut self, pct: f64) -> Self {
        self.ratelimit_retry_after_jitter_pct = pct;
        self
    }

    /// Set a burst size for token-bucket and GCRA algorithms.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_burst(mut self, burst: u64) -> Self {
        self.ratelimit_burst = Some(burst);
        self
    }

    // ── CORS ───────────────────────────────────────────────────────────────────

    /// Override the default permissive CORS layer.
    pub fn with_cors(mut self, cors: CorsLayer) -> Self {
        self.cors = Some(cors);
        self
    }

    /// Configure CORS from a structured [`CorsConfig`].
    pub fn with_cors_config(mut self, cfg: CorsConfig) -> Result<Self> {
        self.cors = Some(crate::adapters::cors::build_cors_layer(&cfg)?);
        Ok(self)
    }

    // ── Router ─────────────────────────────────────────────────────────────────

    /// Provide the router builder closure.
    pub fn with_router<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&BootstrapCtx) -> Router + Send + 'static,
    {
        self.router_builder = Some(Box::new(f));
        self
    }

    // ── OpenAPI ────────────────────────────────────────────────────────────────

    /// Mount an OpenAPI spec and Swagger UI.
    #[cfg(feature = "openapi")]
    pub fn with_openapi(mut self, api: utoipa::openapi::OpenApi) -> Self {
        self.openapi = Some(api);
        self
    }

    /// Override the spec and UI mount paths.
    #[cfg(feature = "openapi")]
    pub fn with_openapi_paths(
        mut self,
        spec_path: impl Into<String>,
        ui_path: impl Into<String>,
    ) -> Self {
        self.openapi_spec_path = spec_path.into();
        self.openapi_ui_path = ui_path.into();
        self
    }

    // ── Auth ───────────────────────────────────────────────────────────────────

    /// Enable JWT/OIDC + API-key authentication.
    #[cfg(feature = "auth")]
    pub fn with_auth(mut self, config: crate::auth::AuthConfig) -> Self {
        self.auth_layer = Some(crate::auth::AuthLayer::new(config));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_bones::health::HealthCheck;

    #[test]
    fn builder_methods_compose() {
        let _b = ServiceBootstrap::new("svc")
            .with_version("1.2.3")
            .with_health_path("/hc")
            .with_body_limit(1024)
            .with_shutdown_timeout(std::time::Duration::from_secs(1))
            .with_cors(CorsLayer::permissive())
            .with_cors_config(CorsConfig {
                allowed_origins: vec!["https://app.example.com".into()],
                allow_credentials: true,
                max_age_secs: Some(600),
                ..Default::default()
            })
            .unwrap()
            .with_readiness_check("noop", || async { HealthCheck::pass("noop") })
            .with_router(|_| Router::new());
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn builder_with_telemetry_sets_flag() {
        let b = ServiceBootstrap::new("svc").with_telemetry();
        assert!(b.telemetry);
    }

    #[tokio::test]
    async fn serve_errors_when_router_missing() {
        let err = ServiceBootstrap::new("x").serve("127.0.0.1:0").await;
        assert!(matches!(err, Err(Error::Config(_))));
    }

    #[tokio::test]
    async fn serve_errors_on_bad_addr() {
        let err = ServiceBootstrap::new("x")
            .with_router(|_| Router::new())
            .serve("not an addr")
            .await;
        assert!(matches!(err, Err(Error::Config(_))));
    }

    #[test]
    fn from_config_applies_all_fields() {
        use crate::config::{BootstrapConfig, RateLimitConfig, RateLimitKind};
        let cfg = BootstrapConfig {
            bind_addr: "127.0.0.1:1234".into(),
            health_path: "/hc".into(),
            body_limit_bytes: 4096,
            shutdown_timeout_secs: 5,
            version: Some("9.9.9".into()),
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Memory {
                    limit: 10,
                    window_secs: 60,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let b = ServiceBootstrap::from_config("svc", cfg).unwrap();
        assert_eq!(b.bind_addr.as_deref(), Some("127.0.0.1:1234"));
        assert_eq!(b.health_path, "/hc");
        assert_eq!(b.body_limit_bytes, 4096);
        assert_eq!(b.shutdown_timeout, std::time::Duration::from_secs(5));
        assert_eq!(b.version, "9.9.9");
    }

    #[test]
    fn with_shutdown_hook_registers_in_order() {
        let b = ServiceBootstrap::new("svc")
            .with_shutdown_hook("first", std::time::Duration::from_secs(5), || async {})
            .with_shutdown_hook("second", std::time::Duration::from_secs(5), || async {});
        assert_eq!(b.shutdown_hooks.len(), 2);
        assert_eq!(b.shutdown_hooks[0].name, "first");
        assert_eq!(b.shutdown_hooks[1].name, "second");
    }
}
