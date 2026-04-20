//! Observability adapters — telemetry initialisation and OpenAPI/Swagger UI.

pub(crate) mod telemetry;

#[cfg(feature = "openapi")]
pub(crate) mod openapi;
