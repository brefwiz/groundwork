// SPDX-License-Identifier: MIT

use chrono::{Duration, Utc};
use socle::bff::session::{PostgresSessionStore, SessionKey, SessionStore};
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

struct TestDb {
    _container: testcontainers::ContainerAsync<Postgres>,
    pool: PgPool,
}

impl TestDb {
    async fn new() -> Self {
        let container = Postgres::default()
            .start()
            .await
            .expect("start postgres container");

        let host = container.get_host().await.expect("get host");
        let port = container.get_host_port_ipv4(5432).await.expect("get port");

        let connection_string = format!("postgresql://postgres:postgres@{host}:{port}/postgres");

        // Connect to database
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect(&connection_string)
            .await
            .expect("connect to test db");

        // Apply migrations
        sqlx::query(
            r"CREATE TABLE bff_sessions (
                session_id   TEXT PRIMARY KEY,
                payload      BYTEA NOT NULL,
                expires_at   TIMESTAMPTZ NOT NULL,
                created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .execute(&pool)
        .await
        .expect("create bff_sessions table");

        sqlx::query(r"CREATE INDEX idx_bff_sessions_expires_at ON bff_sessions (expires_at)")
            .execute(&pool)
            .await
            .expect("create expires_at index");

        TestDb {
            _container: container,
            pool,
        }
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[tokio::test]
async fn issue_put_and_get() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "test-session-123".to_string();
    let value = b"encrypted-payload-data".to_vec();
    let expires_at = Utc::now() + Duration::hours(1);

    // Put session
    store
        .put(&key, value.clone(), expires_at)
        .await
        .expect("put session");

    // Get session
    let retrieved = store.get(&key).await.expect("get session");

    assert_eq!(retrieved, Some(value));
}

#[tokio::test]
async fn validate_unknown_key_returns_none() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "nonexistent-key".to_string();

    let retrieved = store.get(&key).await.expect("get nonexistent key");

    assert_eq!(retrieved, None);
}

#[tokio::test]
async fn consume_single_use() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "consume-test-key".to_string();
    let value = b"single-use-payload".to_vec();
    let expires_at = Utc::now() + Duration::hours(1);

    // Put session
    store
        .put(&key, value.clone(), expires_at)
        .await
        .expect("put session");

    // Consume once
    let retrieved = store.consume(&key).await.expect("consume session");

    assert_eq!(retrieved, Some(value));

    // Second consume should return None (already consumed)
    let second_consume = store.consume(&key).await.expect("consume again");

    assert_eq!(second_consume, None);

    // Get should also return None
    let get_after = store.get(&key).await.expect("get after consume");

    assert_eq!(get_after, None);
}

#[tokio::test]
async fn expire_treats_past_expiry_as_absent() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "expired-key".to_string();
    let value = b"expired-payload".to_vec();
    // Set expiry to the past
    let expires_at = Utc::now() - Duration::hours(1);

    // Put session with past expiry
    store
        .put(&key, value, expires_at)
        .await
        .expect("put expired session");

    // Get should return None (expired)
    let retrieved = store.get(&key).await.expect("get expired session");

    assert_eq!(retrieved, None);

    // Consume should also return None (expired)
    let consumed = store.consume(&key).await.expect("consume expired session");

    assert_eq!(consumed, None);
}
