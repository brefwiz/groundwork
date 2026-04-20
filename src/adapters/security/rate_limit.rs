//! Rate limit adapter — implementation provided by consumers via RateLimitStore trait.
//!
//! TODO: implement rate limiting middleware backed by configurable stores
//! (in-memory, Postgres, Redis). The builder stubs accept config but do not
//! yet install a middleware layer.

/// Backend selection for the rate limiter.
#[derive(Debug, Clone)]
pub enum RateLimitBackend {
    /// In-process memory store.
    Memory {
        /// Max requests per window.
        limit: u32,
        /// Window duration in seconds.
        window_secs: u64,
    },
    /// Postgres-backed store.
    #[cfg(feature = "ratelimit-postgres")]
    Postgres {
        /// Max requests per window.
        limit: u32,
        /// Window duration in seconds.
        window_secs: u64,
    },
    /// Redis-backed store.
    #[cfg(feature = "ratelimit-redis")]
    Redis {
        /// deadpool-redis connection pool.
        pool: deadpool_redis::Pool,
        /// Max requests per window.
        limit: u32,
        /// Window duration in seconds.
        window_secs: u64,
    },
}

impl RateLimitBackend {
    /// Return (limit, window_secs) regardless of backend variant.
    pub fn common_params(&self) -> (u32, u64) {
        match self {
            Self::Memory { limit, window_secs } => (*limit, *window_secs),
            #[cfg(feature = "ratelimit-postgres")]
            Self::Postgres { limit, window_secs } => (*limit, *window_secs),
            #[cfg(feature = "ratelimit-redis")]
            Self::Redis {
                limit, window_secs, ..
            } => (*limit, *window_secs),
        }
    }

    /// Build a Redis backend from a `redis://...` URL.
    #[cfg(feature = "ratelimit-redis")]
    pub fn redis_from_url(
        url: &str,
        limit: u32,
        window_secs: u64,
    ) -> crate::error::Result<Self> {
        let manager = deadpool_redis::Manager::new(url)
            .map_err(|e| crate::error::Error::Config(format!("redis url: {e}")))?;
        let pool = deadpool_redis::Pool::builder(manager)
            .build()
            .map_err(|e| crate::error::Error::Config(format!("redis pool: {e}")))?;
        Ok(Self::Redis {
            pool,
            limit,
            window_secs,
        })
    }
}

/// Key extraction strategy for rate limiting.
#[derive(Debug, Clone, Default)]
pub enum RateLimitExtractor {
    /// Limit by remote IP address.
    #[default]
    Ip,
    /// Limit by `x-user-id` header.
    UserId,
    /// Limit by `x-api-key` header.
    ApiKey,
    /// Limit by `x-org-id` header.
    OrgId,
    /// Limit by an arbitrary header name.
    Header(String),
}
