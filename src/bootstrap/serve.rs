//! `serve()` implementation — wires adapters, binds the listener, runs until
//! shutdown, then drains.

use std::net::SocketAddr;

#[cfg(feature = "prometheus-metrics")]
use axum::Router;
#[cfg(feature = "prometheus-metrics")]
use std::sync::Arc;

use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::bootstrap::builder::ServiceBootstrap;
use crate::bootstrap::ctx::BootstrapCtx;
use crate::error::{Error, Result};

impl ServiceBootstrap {
    /// Run the service. Initialises every enabled integration in dependency
    /// order, binds the listener, serves until SIGINT/SIGTERM, then drains.
    pub async fn serve(self, addr: impl Into<String>) -> Result<()> {
        // 1. Telemetry first.
        #[cfg(feature = "telemetry")]
        if self.telemetry {
            crate::adapters::telemetry::init_basic_tracing();
        } else {
            crate::adapters::telemetry::init_basic_tracing();
        }
        #[cfg(not(feature = "telemetry"))]
        crate::adapters::telemetry::init_basic_tracing();

        // 2. Database pool.
        #[cfg(feature = "database")]
        let db: Option<sqlx::PgPool> = if let Some(ref url) = self.database_url {
            let pool = sqlx::PgPool::connect(url)
                .await
                .map_err(|e| Error::Database(e.to_string()))?;

            if let Some(ref migrator) = self.migrator {
                tracing::warn!(
                    service = %self.service_name,
                    "groundwork: running migrations in-process"
                );
                migrator
                    .run(&pool)
                    .await
                    .map_err(|e| Error::Database(format!("migrate: {e}")))?;
                tracing::info!("groundwork: migrations applied successfully");
            }

            Some(pool)
        } else if self.migrator.is_some() {
            return Err(Error::Config(
                "with_migrations(...) requires with_database(...) to be called first".into(),
            ));
        } else {
            None
        };

        // 3. Build the user router via ctx.
        let ctx = BootstrapCtx {
            service_name: self.service_name.clone(),
            #[cfg(feature = "database")]
            db: db.clone(),
        };

        let router_builder = self
            .router_builder
            .ok_or_else(|| Error::Config("with_router(...) was never called".into()))?;
        let user_router = router_builder(&ctx);

        // 4. Mount health endpoints.
        let health_router = crate::adapters::health::build_health_router(
            &self.health_path,
            &self.service_name,
            &self.version,
            self.readiness_checks.clone(),
        );
        let mut user_router = user_router.merge(health_router);

        // OpenAPI spec + Swagger UI.
        #[cfg(feature = "openapi")]
        if let Some(mut api) = self.openapi.clone() {
            api = crate::adapters::openapi::merge_health_paths(api, &self.health_path);
            user_router = crate::adapters::openapi::mount_openapi(
                user_router,
                api,
                &self.openapi_spec_path,
                &self.openapi_ui_path,
            );
        }

        let user_router = user_router.fallback(crate::adapters::health::not_found_fallback);

        // 5. Apply layers.
        let mut app = user_router;

        // Auth layer.
        #[cfg(feature = "auth")]
        if let Some(auth_layer) = self.auth_layer {
            app = app.layer(auth_layer);
        }

        // Enrich bare error responses.
        app = app.layer(axum::middleware::from_fn(
            crate::adapters::security::enrich_error::enrich_error_response,
        ));

        // Prometheus /metrics endpoint.
        #[cfg(feature = "prometheus-metrics")]
        if self.prometheus_metrics {
            let registry = Arc::new(prometheus::Registry::new());
            let path = self.prometheus_path.clone();
            app = app.merge(
                Router::new()
                    .route(
                        &path,
                        axum::routing::get(
                            crate::adapters::openapi::prometheus_scrape_handler,
                        ),
                    )
                    .with_state(registry),
            );
        }

        // Cross-cutting tower-http layers.
        use tower_http::catch_panic::CatchPanicLayer;
        use tower_http::compression::CompressionLayer;
        use tower_http::limit::RequestBodyLimitLayer;
        use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};

        let request_id_header = axum::http::HeaderName::from_static("x-request-id");
        let cors = self.cors.unwrap_or_else(CorsLayer::permissive);

        let trace_layer =
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                let request_id = crate::request_id::extract_request_id(req);
                tracing::info_span!(
                    "request",
                    method = %req.method(),
                    uri = %req.uri(),
                    "request.id" = request_id,
                )
            });

        app = app
            .layer(CompressionLayer::new())
            .layer(RequestBodyLimitLayer::new(self.body_limit_bytes))
            .layer(cors)
            .layer(CatchPanicLayer::custom(crate::handler_error::panic_handler))
            .layer(trace_layer)
            .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
            .layer(crate::request_id::RequestIdTaskLocalLayer)
            .layer(SetRequestIdLayer::new(
                request_id_header,
                crate::request_id::MakeRequestUuidV7,
            ));

        // 6. Bind & serve with graceful shutdown.
        let addr: SocketAddr = addr
            .into()
            .parse()
            .map_err(|e: std::net::AddrParseError| Error::Config(e.to_string()))?;
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::Bind(e.to_string()))?;
        tracing::info!(%addr, service = %self.service_name, "groundwork: listening");

        let shutdown_timeout = self.shutdown_timeout;
        let shutdown_hooks = self.shutdown_hooks;
        let make_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = axum::serve(listener, make_service).with_graceful_shutdown(shutdown_signal());

        tokio::select! {
            res = server => {
                res.map_err(|e| Error::Serve(e.to_string()))?;
            }
            () = async {
                shutdown_signal().await;
                tokio::time::sleep(shutdown_timeout).await;
            } => {
                tracing::error!(
                    timeout_secs = shutdown_timeout.as_secs(),
                    "groundwork: shutdown deadline exceeded"
                );
            }
        }

        // 7. Run user-registered shutdown hooks in reverse registration order.
        for hook in shutdown_hooks.into_iter().rev() {
            tracing::info!(hook = %hook.name, "groundwork: running shutdown hook");
            match tokio::time::timeout(hook.timeout, (hook.hook)()).await {
                Ok(()) => {
                    tracing::info!(hook = %hook.name, "groundwork: shutdown hook completed");
                }
                Err(_) => {
                    tracing::error!(
                        hook = %hook.name,
                        timeout_secs = hook.timeout.as_secs(),
                        "groundwork: shutdown hook timed out"
                    );
                }
            }
        }

        tracing::info!("groundwork: shutdown complete");
        Ok(())
    }
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
