// SPDX-License-Identifier: MIT

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::types::PgInterval;
use sqlx::{PgPool, Row};

use crate::bff::session::ports::{SessionKey, SessionRenewal, SessionStore, SessionStoreError};

/// Plain-`sqlx` Postgres implementation of [`SessionStore`] — no
/// sqlx-switchboard dependency, safe for the OSS floor.
pub struct PostgresSessionStore {
    pool: PgPool,
}

impl PostgresSessionStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStore for PostgresSessionStore {
    async fn put(
        &self,
        key: &SessionKey,
        value: Vec<u8>,
        expires_at: DateTime<Utc>,
    ) -> Result<(), SessionStoreError> {
        sqlx::query(
            r"
            INSERT INTO bff_sessions (session_id, payload, expires_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id) DO UPDATE
                SET payload = EXCLUDED.payload, expires_at = EXCLUDED.expires_at
            ",
        )
        .bind(key)
        .bind(value)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionStoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let row = sqlx::query(
            r"SELECT payload FROM bff_sessions WHERE session_id = $1 AND expires_at > now()",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionStoreError::Backend(e.to_string()))?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("payload")))
    }

    async fn delete(&self, key: &SessionKey) -> Result<(), SessionStoreError> {
        sqlx::query(r"DELETE FROM bff_sessions WHERE session_id = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| SessionStoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn consume(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let row = sqlx::query(
            r"
            DELETE FROM bff_sessions
            WHERE session_id = $1 AND expires_at > now()
            RETURNING payload
            ",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionStoreError::Backend(e.to_string()))?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("payload")))
    }

    async fn touch_if_stale(
        &self,
        key: &SessionKey,
        new_expires_at: DateTime<Utc>,
        min_delta: Duration,
        absolute_cap: DateTime<Utc>,
    ) -> Result<SessionRenewal, SessionStoreError> {
        let min_delta = PgInterval::try_from(min_delta)
            .map_err(|e| SessionStoreError::Backend(format!("invalid renewal threshold: {e}")))?;

        // One statement carries every rule: the liveness check, the cap clamp,
        // and the threshold. Reading the stored deadline into Rust to compare
        // it would open a lost-update window between concurrent requests on the
        // same key — LEAST() and the predicate both run against the row the
        // UPDATE locks. The threshold is measured against the clamped value,
        // not the proposed one, so a record already at the cap stops writing.
        let row = sqlx::query(
            r"
            UPDATE bff_sessions
            SET expires_at = LEAST($2, $3)
            WHERE session_id = $1
              AND expires_at > now()
              AND LEAST($2, $3) - expires_at >= $4
            RETURNING expires_at
            ",
        )
        .bind(key)
        .bind(new_expires_at)
        .bind(absolute_cap)
        .bind(min_delta)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionStoreError::Backend(e.to_string()))?;

        Ok(row.map_or(SessionRenewal::NotRenewed, |r| {
            SessionRenewal::Renewed(r.get::<DateTime<Utc>, _>("expires_at"))
        }))
    }
}
