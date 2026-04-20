//! Health endpoint adapter.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};

use crate::ports::health::ReadinessCheckFn;

/// Build the health sub-router mounted under `base` (default `/health`).
pub(crate) fn build_health_router(
    base: &str,
    service_id: &str,
    version: &str,
    checks: Vec<(String, ReadinessCheckFn)>,
) -> Router {
    use api_bones::health::{HealthCheck, HealthStatus, LivenessResponse};

    let service_id = service_id.to_string();
    let version = version.to_string();

    let live_path = format!("{base}/live");
    let ready_path = format!("{base}/ready");

    let checks = Arc::new(checks);

    Router::new()
        .route(
            &live_path,
            get(move || {
                let body = LivenessResponse::pass(version.clone(), service_id.clone());
                async move { (StatusCode::OK, Json(body)).into_response() }
            }),
        )
        .route(
            &ready_path,
            get(move || {
                let checks = checks.clone();
                async move {
                    let mut results: Vec<HealthCheck> = Vec::with_capacity(checks.len());
                    let mut worst = HealthStatus::Pass;
                    for (_name, check) in checks.iter() {
                        let result = check().await;
                        worst = worst_of(worst, result.status.clone());
                        results.push(result);
                    }
                    let mut by_name: HashMap<String, Vec<HealthCheck>> = HashMap::new();
                    for ((name, _), result) in checks.iter().zip(results.into_iter()) {
                        by_name.entry(name.clone()).or_default().push(result);
                    }
                    let status = StatusCode::from_u16(worst.http_status())
                        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                    (status, Json(by_name)).into_response()
                }
            }),
        )
}

/// Fallback handler for unmatched routes. Returns RFC 9457 Problem+JSON 404.
pub(crate) async fn not_found_fallback(req: axum::extract::Request) -> axum::response::Response {
    use api_bones::ApiError;
    let path = req.uri().path().to_string();
    let rid = crate::request_id::extract_request_id(&req);
    let mut err = ApiError::not_found(format!("no route for {path}"));
    if let Ok(uuid) = uuid::Uuid::parse_str(rid) {
        err = err.with_request_id(uuid);
    }
    err.into_response()
}

fn worst_of(
    a: api_bones::health::HealthStatus,
    b: api_bones::health::HealthStatus,
) -> api_bones::health::HealthStatus {
    use api_bones::health::HealthStatus::*;
    match (a, b) {
        (Fail, _) | (_, Fail) => Fail,
        (Warn, _) | (_, Warn) => Warn,
        _ => Pass,
    }
}
