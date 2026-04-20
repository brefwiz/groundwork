//! Security adapters — CORS layer and rate limiting.

pub(crate) mod cors;
pub(crate) mod enrich_error;

// Rate limit adapter — implementation provided by consumers via RateLimitStore trait.
// See ports::secret_vault.
// TODO: implement rate limit adapter backed by configurable stores.
pub(crate) mod rate_limit;
