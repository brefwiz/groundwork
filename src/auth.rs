//! JWT/OIDC + API-key authentication middleware for Axum.
//!
//! Enabled by the `auth` feature.

// TODO: implement JWT/OIDC authentication layer.
// This module is a stub — provide AuthConfig, AuthLayer, and Claims types.

use axum::{
    body::Body,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

/// Configuration for the authentication layer.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// JWKS endpoint URL.
    pub jwks_url: String,
    /// Expected token issuer.
    pub issuer: String,
    /// Expected audiences.
    pub audience: Vec<String>,
}

/// Verified JWT claims extracted from a valid bearer token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID).
    pub sub: String,
    /// Issuer.
    pub iss: Option<String>,
    /// Audiences.
    pub aud: Option<Vec<String>>,
    /// Expiry (Unix timestamp).
    pub exp: Option<u64>,
}

impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Claims>()
            .cloned()
            .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
    }
}

/// Tower layer that validates inbound JWTs.
#[derive(Clone)]
pub struct AuthLayer {
    config: AuthConfig,
}

impl AuthLayer {
    /// Create a new auth layer with the given configuration.
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }
}

impl<S> tower::Layer<S> for AuthLayer {
    type Service = AuthService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            _config: self.config.clone(),
        }
    }
}

/// Service produced by [`AuthLayer`].
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    _config: AuthConfig,
}

impl<S> tower::Service<axum::http::Request<Body>> for AuthService<S>
where
    S: tower::Service<axum::http::Request<Body>, Response = Response> + Clone + Send + 'static,
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

    fn call(&mut self, req: axum::http::Request<Body>) -> Self::Future {
        // TODO: validate JWT, insert Claims into extensions on success.
        let fut = self.inner.call(req);
        Box::pin(fut)
    }
}
