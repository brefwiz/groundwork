//! Enrich bare error responses with a Problem+JSON body.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use api_bones::error::{ApiError, ErrorCode};

/// Axum middleware: enriches bare error responses with a Problem+JSON body.
pub async fn enrich_error_response(req: Request<Body>, next: Next) -> Response {
    let resp = next.run(req).await;

    let status = resp.status();

    if !status.is_client_error() && !status.is_server_error() {
        return resp;
    }

    if resp.headers().get(header::CONTENT_TYPE).is_some() {
        return resp;
    }

    let (parts, _body) = resp.into_parts();

    let err = match status {
        StatusCode::UNAUTHORIZED => ApiError::unauthorized("Authentication required"),
        StatusCode::FORBIDDEN => ApiError::forbidden("Insufficient permissions"),
        other => ApiError::new(
            ErrorCode::InternalServerError,
            other.canonical_reason().unwrap_or("Unexpected error"),
        ),
    };

    let mut new_resp = err.into_response();
    for (name, value) in parts
        .headers
        .into_iter()
        .flat_map(|(n, v)| n.map(|n| (n, v)))
    {
        if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
            new_resp.headers_mut().insert(name, value);
        }
    }
    *new_resp.status_mut() = status;
    new_resp
}
