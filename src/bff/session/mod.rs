// SPDX-License-Identifier: MIT

//! BFF session TCB module.
//!
//! Layout:
//! - [`ports`] — the transport-free port traits ([`SessionStore`],
//!   [`EnvelopeCrypto`], [`KekSource`]) that carry no KMS, transport, or
//!   product-specific dependency.
//! - [`crypto`] — the single AEAD envelope-crypto body over an injected
//!   [`KekSource`].
//! - [`env_kek`] — the community, operator-managed [`KekSource`].
//! - [`postgres_store`] — the plain-`sqlx` [`SessionStore`] implementation.

pub mod crypto;
pub mod env_kek;
pub mod ports;
pub mod postgres_store;

pub use ports::{
    EnvelopeCrypto, EnvelopeCryptoError, KekError, KekSource, SessionKey, SessionStore,
    SessionStoreError,
};

pub use crypto::AeadEnvelopeCrypto;
pub use env_kek::EnvKekSource;
pub use postgres_store::PostgresSessionStore;

#[cfg(any(test, feature = "test-crypto"))]
pub use crypto::InProcessTestKekSource;

/// Migrator carrying the `bff_sessions` schema that backs
/// [`PostgresSessionStore`].
///
/// The `PostgresSessionStore` is only a thin `sqlx` wrapper over an existing
/// table — it does not create its schema. Consumers that use the Postgres store
/// must run this migrator against their pool so the table exists. The
/// recommended path is the opt-in bootstrap flag
/// [`ServiceBootstrap::with_bff_session_migrations`], which invokes
/// [`run_session_migrations`] after the consumer's own migrator; callers that
/// build their own pool can call [`run_session_migrations`] directly.
///
/// [`ServiceBootstrap::with_bff_session_migrations`]: crate::ServiceBootstrap::with_bff_session_migrations
pub static SESSION_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// Isolated migrations-history table for [`SESSION_MIGRATOR`].
///
/// The session migrator shares a pool with the consumer's own migrator, but it
/// must NOT share the default `_sqlx_migrations` history table: the consumer's
/// migrator runs with `ignore_missing = false` (the sqlx default) and would
/// reject the session migration's row as an unknown version on the next
/// startup. A dedicated history table keeps each migrator's bookkeeping
/// disjoint, so neither ever observes the other's rows.
const SESSION_MIGRATIONS_TABLE: &str = "_sqlx_bff_session_migrations";

/// Run the BFF session schema migration against `pool`.
///
/// Idempotent — safe to call on every startup. Uses a dedicated migrations
/// history table (see [`SESSION_MIGRATIONS_TABLE`]) so it composes cleanly with
/// a consumer's own migrator on the same pool.
///
/// # Errors
///
/// Returns the underlying `sqlx` migration error if applying the schema fails.
pub async fn run_session_migrations(
    pool: &sqlx::PgPool,
) -> Result<(), sqlx::migrate::MigrateError> {
    let mut migrator = sqlx::migrate::Migrator {
        migrations: SESSION_MIGRATOR.migrations.clone(),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    migrator.dangerous_set_table_name(SESSION_MIGRATIONS_TABLE);
    migrator.run(pool).await
}
