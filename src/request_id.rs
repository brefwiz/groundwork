//! Request-ID / correlation-ID middleware.

use axum::http::{HeaderValue, Request};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

tokio::task_local! {
    /// The request-ID string for the current request task.
    pub(crate) static CURRENT_REQUEST_ID: String;
}

/// [`MakeRequestId`] implementation that generates sortable UUIDv7 identifiers
/// and accepts inbound `x-request-id` / `x-correlation-id` headers.
#[derive(Clone, Default)]
pub struct MakeRequestUuidV7;

impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, request: &Request<B>) -> Option<RequestId> {
        let headers = request.headers();

        if headers.contains_key("x-request-id") {
            return None;
        }

        if let Some(v) = headers.get("x-correlation-id") {
            return Some(RequestId::new(v.clone()));
        }

        let id = Uuid::now_v7().to_string();
        Some(RequestId::new(
            HeaderValue::from_str(&id).expect("UUIDv7 is a valid header value"),
        ))
    }
}

/// Extract the request-ID string from `request` extensions.
pub(crate) fn extract_request_id<B>(request: &Request<B>) -> &str {
    request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or("")
}

/// Tower [`Layer`] that stores the current request's ID in a task-local variable.
#[derive(Clone, Default)]
pub(crate) struct RequestIdTaskLocalLayer;

impl<S> tower::Layer<S> for RequestIdTaskLocalLayer {
    type Service = RequestIdTaskLocalService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdTaskLocalService { inner }
    }
}

/// Service produced by [`RequestIdTaskLocalLayer`].
#[derive(Clone)]
pub(crate) struct RequestIdTaskLocalService<S> {
    inner: S,
}

impl<S, ReqBody> tower::Service<Request<ReqBody>> for RequestIdTaskLocalService<S>
where
    S: tower::Service<Request<ReqBody>>,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let id = extract_request_id(&req).to_owned();
        let fut = self.inner.call(req);
        Box::pin(CURRENT_REQUEST_ID.scope(id, fut))
    }
}
