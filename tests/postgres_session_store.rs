// SPDX-License-Identifier: MIT

use chrono::{DateTime, Duration, SubsecRound, Utc};
use socle::bff::session::{PostgresSessionStore, SessionKey, SessionRenewal, SessionStore};
use sqlx::{PgPool, Row};
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

/// `timestamptz` keeps microseconds, so a deadline that round-trips through
/// Postgres comes back with its nanosecond tail truncated. Every deadline these
/// tests compare against is anchored here, so an equality assertion measures the
/// renewal logic rather than the column's resolution.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn renewed_at(outcome: SessionRenewal) -> DateTime<Utc> {
    match outcome {
        SessionRenewal::Renewed(at) => at,
        SessionRenewal::NotRenewed => panic!("expected the record to be renewed"),
    }
}

#[tokio::test]
async fn touch_beyond_threshold_extends_deadline() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "touch-extends".to_string();
    let stored_deadline = now() + Duration::minutes(10);
    store
        .put(&key, b"payload".to_vec(), stored_deadline)
        .await
        .expect("put session");

    let proposed = now() + Duration::minutes(30);
    let outcome = store
        .touch_if_stale(
            &key,
            proposed,
            Duration::minutes(1),
            now() + Duration::hours(24),
        )
        .await
        .expect("touch session");

    let renewed = renewed_at(outcome);
    assert!(
        renewed > stored_deadline,
        "renewal must move the deadline forward: {renewed} <= {stored_deadline}"
    );
}

#[tokio::test]
async fn touch_within_threshold_leaves_deadline_untouched() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "touch-below-threshold".to_string();
    let stored_deadline = now() + Duration::minutes(10);
    store
        .put(&key, b"payload".to_vec(), stored_deadline)
        .await
        .expect("put session");

    // Ten seconds beyond the stored deadline, against a five-minute threshold.
    let outcome = store
        .touch_if_stale(
            &key,
            stored_deadline + Duration::seconds(10),
            Duration::minutes(5),
            now() + Duration::hours(24),
        )
        .await
        .expect("touch session");

    assert_eq!(outcome, SessionRenewal::NotRenewed);

    let actual: DateTime<Utc> =
        sqlx::query(r"SELECT expires_at FROM bff_sessions WHERE session_id = $1")
            .bind(&key)
            .fetch_one(db.pool())
            .await
            .expect("read back deadline")
            .get("expires_at");
    assert_eq!(actual, stored_deadline);
}

#[tokio::test]
async fn touch_absent_key_reports_not_renewed() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let outcome = store
        .touch_if_stale(
            &"never-existed".to_string(),
            now() + Duration::minutes(30),
            Duration::minutes(1),
            now() + Duration::hours(24),
        )
        .await
        .expect("touch absent key");

    assert_eq!(outcome, SessionRenewal::NotRenewed);
}

#[tokio::test]
async fn touch_never_resurrects_a_lapsed_record() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "touch-lapsed".to_string();
    let lapsed_deadline = now() - Duration::hours(1);
    store
        .put(&key, b"payload".to_vec(), lapsed_deadline)
        .await
        .expect("put lapsed session");

    let outcome = store
        .touch_if_stale(
            &key,
            now() + Duration::minutes(30),
            Duration::minutes(1),
            now() + Duration::hours(24),
        )
        .await
        .expect("touch lapsed session");

    assert_eq!(outcome, SessionRenewal::NotRenewed);
    assert_eq!(store.get(&key).await.expect("get lapsed"), None);

    let actual: DateTime<Utc> =
        sqlx::query(r"SELECT expires_at FROM bff_sessions WHERE session_id = $1")
            .bind(&key)
            .fetch_one(db.pool())
            .await
            .expect("read back deadline")
            .get("expires_at");
    assert_eq!(actual, lapsed_deadline);
}

#[tokio::test]
async fn touch_clamps_to_the_absolute_cap() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "touch-clamped".to_string();
    store
        .put(&key, b"payload".to_vec(), now() + Duration::minutes(10))
        .await
        .expect("put session");

    let cap = now() + Duration::hours(2);
    let outcome = store
        .touch_if_stale(&key, cap + Duration::hours(48), Duration::minutes(1), cap)
        .await
        .expect("touch session");

    assert_eq!(renewed_at(outcome), cap);

    // At the cap, further renewal has nothing left to give and must stop
    // writing rather than rewrite the same value on every request.
    let second = store
        .touch_if_stale(&key, cap + Duration::hours(48), Duration::minutes(1), cap)
        .await
        .expect("touch session at cap");
    assert_eq!(second, SessionRenewal::NotRenewed);
}

#[tokio::test]
async fn concurrent_touches_leave_a_coherent_deadline() {
    let db = TestDb::new().await;
    let store = std::sync::Arc::new(PostgresSessionStore::new(db.pool().clone()));

    let key: SessionKey = "touch-concurrent".to_string();
    store
        .put(&key, b"payload".to_vec(), now() + Duration::minutes(10))
        .await
        .expect("put session");

    let cap = now() + Duration::hours(24);
    let proposed = now() + Duration::minutes(30);

    let left = {
        let (store, key) = (store.clone(), key.clone());
        tokio::spawn(async move {
            store
                .touch_if_stale(&key, proposed, Duration::minutes(1), cap)
                .await
        })
    };
    let right = {
        let (store, key) = (store.clone(), key.clone());
        tokio::spawn(async move {
            store
                .touch_if_stale(&key, proposed, Duration::minutes(1), cap)
                .await
        })
    };

    left.await.expect("join left").expect("left touch");
    right.await.expect("join right").expect("right touch");

    let actual: DateTime<Utc> =
        sqlx::query(r"SELECT expires_at FROM bff_sessions WHERE session_id = $1")
            .bind(&key)
            .fetch_one(db.pool())
            .await
            .expect("read back deadline")
            .get("expires_at");
    assert_eq!(
        actual, proposed,
        "concurrent renewal must not tear the deadline"
    );
    assert!(actual <= cap, "concurrent renewal must respect the cap");
}

#[tokio::test]
async fn touch_never_disturbs_payload_or_creation_time() {
    let db = TestDb::new().await;
    let store = PostgresSessionStore::new(db.pool().clone());

    let key: SessionKey = "touch-preserves".to_string();
    let payload = b"sealed-ciphertext".to_vec();
    store
        .put(&key, payload.clone(), now() + Duration::minutes(10))
        .await
        .expect("put session");

    let before = sqlx::query(r"SELECT payload, created_at FROM bff_sessions WHERE session_id = $1")
        .bind(&key)
        .fetch_one(db.pool())
        .await
        .expect("read before");
    let created_before: DateTime<Utc> = before.get("created_at");

    store
        .touch_if_stale(
            &key,
            now() + Duration::minutes(30),
            Duration::minutes(1),
            now() + Duration::hours(24),
        )
        .await
        .expect("touch session");

    let after = sqlx::query(r"SELECT payload, created_at FROM bff_sessions WHERE session_id = $1")
        .bind(&key)
        .fetch_one(db.pool())
        .await
        .expect("read after");

    assert_eq!(after.get::<Vec<u8>, _>("payload"), payload);
    assert_eq!(after.get::<DateTime<Utc>, _>("created_at"), created_before);
}
