//! Bootstrap dependency-injection context passed to the user's router builder.

use std::sync::Arc;

/// Context handed to the user's router builder closure.
#[derive(Clone)]
pub struct BootstrapCtx {
    pub(crate) service_name: Arc<str>,
    #[cfg(feature = "database")]
    pub(crate) db: Option<sqlx::PgPool>,
}

impl BootstrapCtx {
    /// The service name passed to [`crate::bootstrap::ServiceBootstrap::new`].
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    /// The database pool, if `with_database` was called.
    ///
    /// Panics if called without `with_database()` — that's intentional: a
    /// missing pool is a wiring bug, not a runtime condition.
    #[cfg(feature = "database")]
    pub fn db(&self) -> &sqlx::PgPool {
        self.db
            .as_ref()
            .expect("BootstrapCtx::db called but with_database() was never invoked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_is_accessible() {
        let ctx = BootstrapCtx {
            service_name: Arc::from("svc"),
            #[cfg(feature = "database")]
            db: None,
        };
        assert_eq!(ctx.service_name(), "svc");
    }

    #[cfg(feature = "database")]
    #[test]
    #[should_panic(expected = "BootstrapCtx::db called")]
    fn db_panics_when_missing() {
        let ctx = BootstrapCtx {
            service_name: Arc::from("svc"),
            db: None,
        };
        let _ = ctx.db();
    }

    #[test]
    fn clone_preserves_service_name() {
        let ctx = BootstrapCtx {
            service_name: Arc::from("my-service"),
            #[cfg(feature = "database")]
            db: None,
        };
        let cloned = ctx.clone();
        assert_eq!(cloned.service_name(), "my-service");
    }
}
