//! RED metrics middleware for Prometheus.
//!
//! Enabled by the `prometheus-metrics` feature.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use axum::{body::Body, extract::MatchedPath};
use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram, MeterProvider as _},
};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use tower::{Layer, Service};

use crate::error::{Error, Result};

fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            if seg.parse::<u64>().is_ok() || is_uuid_like(seg) {
                "{id}"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_uuid_like(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, &c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
}

/// Tower layer that records RED metrics via OpenTelemetry.
#[derive(Clone)]
pub struct MetricsLayer {
    requests_total: Counter<u64>,
    request_duration: Histogram<f64>,
}

impl MetricsLayer {
    /// Create a new metrics layer backed by the given Prometheus registry.
    pub fn new(registry: prometheus::Registry) -> Result<Self> {
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry)
            .build()
            .map_err(|e| Error::Config(format!("prometheus exporter: {e}")))?;
        let provider = SdkMeterProvider::builder()
            .with_reader(exporter)
            .build();
        global::set_meter_provider(provider);
        let meter = global::meter("groundwork");
        Ok(Self {
            requests_total: meter
                .u64_counter("http_server_requests_total")
                .with_description("Total HTTP requests")
                .build(),
            request_duration: meter
                .f64_histogram("http_server_request_duration_seconds")
                .with_description("HTTP request duration in seconds")
                .build(),
        })
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            requests_total: self.requests_total.clone(),
            request_duration: self.request_duration.clone(),
        }
    }
}

/// Service produced by [`MetricsLayer`].
#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
    requests_total: Counter<u64>,
    request_duration: Histogram<f64>,
}

impl<S> Service<axum::http::Request<Body>> for MetricsService<S>
where
    S: Service<axum::http::Request<Body>, Response = axum::response::Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<Body>) -> Self::Future {
        let method = req.method().to_string();
        let path = req
            .extensions()
            .get::<MatchedPath>()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| normalize_path(req.uri().path()));

        let start = Instant::now();
        let requests_total = self.requests_total.clone();
        let request_duration = self.request_duration.clone();
        let fut = self.inner.call(req);

        Box::pin(async move {
            let resp = fut.await?;
            let status = resp.status().as_u16().to_string();
            let elapsed = start.elapsed().as_secs_f64();
            let labels = [
                KeyValue::new("method", method),
                KeyValue::new("path", path),
                KeyValue::new("status", status),
            ];
            requests_total.add(1, &labels);
            request_duration.record(elapsed, &labels);
            Ok(resp)
        })
    }
}

/// Create or retrieve a named counter from the global OpenTelemetry meter.
pub fn counter(name: &'static str) -> opentelemetry::metrics::Counter<u64> {
    global::meter("groundwork").u64_counter(name).build()
}
