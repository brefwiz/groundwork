//! Outbound HTTP client with request-ID propagation.
//!
//! Enabled by the `http-client` feature.

// TODO: implement outbound http client middleware that propagates x-request-id
// from the task-local CURRENT_REQUEST_ID set by the request ID layer.
