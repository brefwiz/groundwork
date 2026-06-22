---
service: socle
wire_surface: proto-source
proto_packages: []
openapi_path: ~
capability_exposes: [http-bootstrap, response-builders, ratelimit]
capability_consumes: []
sdk_languages: [rust]
migration_baseline:
  utoipa_handler_count: 0
  baseline_commit: ~
surface_kind: sdk
migration_priority: ~
migration_eta: ~
ci_snowflakes: []
publishes: [rust-crates]
inline_jobs: [resolve-tag]
version_ecosystem: rust
---

# socle — Development Spec

socle is a Rust library crate providing the shared HTTP service bootstrap (axum
wiring, graceful shutdown, health/readiness, body-limit + CORS layers) and the
canonical `ApiResponse<T>` / `HandlerError` builders (ADR platform/0020). It has
no wire surface of its own — every HTTP surface is owned by the consumer services
that depend on it. Per ADR-0086, library crates with no wire surface are clean
under `wire_surface: proto-source`: zero utoipa handlers, no migration debt.
