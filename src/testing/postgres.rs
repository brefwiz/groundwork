use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

/// A running Postgres Docker container. The container is stopped and removed when this struct is dropped.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "testing-postgres")]
/// # mod example {
/// use socle::testing::postgres::EphemeralPostgres;
///
/// #[tokio::test]
/// async fn with_real_database() {
///     let pg = EphemeralPostgres::start().await;
///     let pool = pg.pool().await;
///     let (n,): (i64,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
///     assert_eq!(n, 1);
/// }
/// # }
/// ```
pub struct EphemeralPostgres {
    _container: ContainerAsync<Postgres>,
    connection_url: String,
}

impl EphemeralPostgres {
    /// Pull (if needed) and start a Postgres 16 container on a random host port.
    ///
    /// # Panics
    ///
    /// Panics if the container fails to start or the mapped port cannot be retrieved.
    pub async fn start() -> Self {
        use testcontainers::runners::AsyncRunner;

        let container = Postgres::default()
            .start()
            .await
            .expect("failed to start Postgres container");

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to get mapped port");

        let connection_url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

        Self {
            _container: container,
            connection_url,
        }
    }

    /// Return the Postgres connection URL for this container.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Open a `sqlx` connection pool to this container.
    ///
    /// testcontainers reports the container ready from its startup log line, but
    /// on a CPU/Docker-loaded CI host the mapped port can take longer than
    /// `sqlx`'s default 30s acquire-timeout to begin accepting TCP connections —
    /// a single `connect` then fails with `PoolTimedOut` even though the
    /// container is healthy. Retry the connect with linear backoff so a
    /// slow-to-accept container is waited out; a genuinely dead container still
    /// panics after the attempts are exhausted.
    ///
    /// # Panics
    ///
    /// Panics if the connection cannot be established after several attempts.
    pub async fn pool(&self) -> PgPool {
        use std::time::Duration;

        let mut last_err = None;
        for attempt in 1..=6u32 {
            match sqlx::postgres::PgPoolOptions::new()
                .acquire_timeout(Duration::from_secs(30))
                .connect(&self.connection_url)
                .await
            {
                Ok(pool) => return pool,
                Err(err) => {
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(u64::from(attempt) * 500)).await;
                }
            }
        }
        panic!(
            "failed to connect to ephemeral Postgres after 6 attempts: {}",
            last_err.expect("the retry loop always executes at least once"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn start_pool_select_one() {
        let pg = EphemeralPostgres::start().await;
        let pool = pg.pool().await;
        let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }
}
