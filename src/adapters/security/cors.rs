//! CORS layer builder — adapter over `tower_http::cors`.

use axum::http::{HeaderName, HeaderValue, Method};
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::CorsConfig;
use crate::error::{Error, Result};

pub(crate) fn build_cors_layer(cfg: &CorsConfig) -> Result<CorsLayer> {
    let methods = cfg
        .allowed_methods
        .iter()
        .map(|m| {
            Method::from_bytes(m.as_bytes())
                .map_err(|e| Error::Config(format!("cors: invalid method {m:?}: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let parse_headers = |hs: &[String]| -> Result<Vec<HeaderName>> {
        hs.iter()
            .map(|h| {
                HeaderName::try_from(h.as_str())
                    .map_err(|e| Error::Config(format!("cors: invalid header {h:?}: {e}")))
            })
            .collect()
    };
    let allowed_headers = parse_headers(&cfg.allowed_headers)?;
    let expose_headers = parse_headers(&cfg.expose_headers)?;

    let mut layer = CorsLayer::new()
        .allow_methods(methods)
        .allow_headers(allowed_headers)
        .expose_headers(expose_headers)
        .allow_credentials(cfg.allow_credentials);

    if cfg.allowed_origins.iter().any(|o| o == "*") {
        layer = layer.allow_origin(AllowOrigin::any());
    } else {
        let origins = cfg
            .allowed_origins
            .iter()
            .map(|o| {
                HeaderValue::from_str(o)
                    .map_err(|e| Error::Config(format!("cors: invalid origin {o:?}: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        layer = layer.allow_origin(origins);
    }

    if let Some(secs) = cfg.max_age_secs {
        layer = layer.max_age(Duration::from_secs(secs));
    }

    Ok(layer)
}
