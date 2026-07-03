// SPDX-License-Identifier: MIT

//! Verifies that the BFF session migrator provisions the `bff_sessions` schema
//! on a fresh database and composes cleanly with a consumer's own migrator on
//! the same pool (dedicated history table, no `_sqlx_migrations` collision).

use chrono::{Duration, Utc};
use socle::bff::session::{PostgresSessionStore, SessionKey, SessionStore, run_session_migrations};
use sqlx::{PgPool, Row};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

async fn fresh_pool() -> (testcontainers::ContainerAsync<Postgres>, PgPool) {
    let container = Postgres::default()
        .start()
        .await
        .expect("start postgres container");

    let host = container.get_host().await.expect("get host");
    let port = container.get_host_port_ipv4(5432).await.expect("get port");
    let connection_string = format!("postgresql://postgres:postgres@{host}:{port}/postgres");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&connection_string)
        .await
        .expect("connect to test db");

    (container, pool)
}

async fn table_exists(pool: &PgPool, table: &str) -> bool {
    let row = sqlx::query(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = $1
        ) AS present",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .expect("query information_schema");
    row.get::<bool, _>("present")
}

#[tokio::test]
async fn migrator_provisions_schema_and_store_round_trips() {
    let (_container, pool) = fresh_pool().await;

    // Fresh DB: the session table does not exist yet.
    assert!(
        !table_exists(&pool, "bff_sessions").await,
        "bff_sessions must not exist before migration"
    );

    // Run the migrator — exactly what `with_bff_session_migrations()` triggers.
    run_session_migrations(&pool)
        .await
        .expect("run bff session migrations");

    // Schema is now present.
    assert!(
        table_exists(&pool, "bff_sessions").await,
        "bff_sessions must exist after migration"
    );

    // The migrator is idempotent: a second run is a no-op, not an error.
    run_session_migrations(&pool)
        .await
        .expect("re-run bff session migrations (idempotent)");

    // The store round-trips against the provisioned schema.
    let store = PostgresSessionStore::new(pool.clone());
    let key: SessionKey = "round-trip-key".to_string();
    let value = b"encrypted-session-payload".to_vec();
    let expires_at = Utc::now() + Duration::hours(1);

    store
        .put(&key, value.clone(), expires_at)
        .await
        .expect("put session");

    let retrieved = store.get(&key).await.expect("get session");
    assert_eq!(retrieved, Some(value));
}

#[tokio::test]
async fn migrator_uses_dedicated_history_table() {
    let (_container, pool) = fresh_pool().await;

    run_session_migrations(&pool)
        .await
        .expect("run bff session migrations");

    // The session migrator records its bookkeeping in its own history table and
    // never touches the consumer's default `_sqlx_migrations` — this isolation
    // is what lets a consumer's own migrator (ignore_missing = false) keep
    // running on the same pool without choking on an unknown version.
    assert!(
        table_exists(&pool, "_sqlx_bff_session_migrations").await,
        "dedicated history table must exist"
    );
    assert!(
        !table_exists(&pool, "_sqlx_migrations").await,
        "session migrator must not create the consumer's default history table"
    );
}
