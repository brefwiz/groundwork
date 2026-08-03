// SPDX-License-Identifier: MIT

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

/// Opaque key used to address a stored session record.
pub type SessionKey = String;

/// Errors returned by [`SessionStore`] operations.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("session not found")]
    NotFound,
    #[error("session store backend error: {0}")]
    Backend(String),
}

/// Errors returned by [`KekSource`] operations.
#[derive(Debug, Error)]
pub enum KekError {
    #[error("kek wrap failed: {0}")]
    Wrap(String),
    #[error("kek unwrap failed: {0}")]
    Unwrap(String),
}

/// Errors returned by [`EnvelopeCrypto`] operations.
#[derive(Debug, Error)]
pub enum EnvelopeCryptoError {
    #[error("envelope seal failed: {0}")]
    Seal(String),
    #[error("envelope open failed: {0}")]
    Open(String),
    #[error(transparent)]
    Kek(#[from] KekError),
}

/// Outcome of a [`SessionStore::touch_if_stale`] renewal attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRenewal {
    /// The record's deadline was extended. Carries the deadline now stored,
    /// which may be earlier than the one proposed if the cap clamped it.
    Renewed(DateTime<Utc>),
    /// The record was left untouched: absent, already past its deadline, or
    /// the proposed extension did not clear the caller's threshold.
    NotRenewed,
}

/// Server-side store for opaque, cookie-keyed BFF session records.
///
/// Implementations own persistence only. The stored `value` is always the
/// already envelope-encrypted blob produced by [`EnvelopeCrypto::seal`] —
/// the store itself is zero-knowledge of plaintext.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Persist `value` under `key` with an absolute `expires_at`, replacing
    /// any existing record.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage
    /// fails, or other variants for specific failure modes.
    async fn put(
        &self,
        key: &SessionKey,
        value: Vec<u8>,
        expires_at: DateTime<Utc>,
    ) -> Result<(), SessionStoreError>;

    /// Fetch the record stored under `key`, if any. Expired records are
    /// treated as absent (`Ok(None)`), never returned.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage fails.
    async fn get(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError>;

    /// Remove the record stored under `key`. Idempotent — deleting a
    /// missing key is not an error.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage fails.
    async fn delete(&self, key: &SessionKey) -> Result<(), SessionStoreError>;

    /// Atomically fetch and delete the record stored under `key`, in one
    /// operation. Expired records are treated as absent (`Ok(None)`), never
    /// returned. This is the single-use consume primitive used for one-shot
    /// tokens (e.g. authorization-code exchange).
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage fails.
    async fn consume(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError>;

    /// Extend the deadline of a live record, without touching its payload.
    ///
    /// This is the renewal primitive behind idle-based expiry: a caller that
    /// treats the stored deadline as an inactivity deadline calls this on each
    /// request to push it forward while the holder stays active.
    ///
    /// Renewal is refused — `NotRenewed`, not an error — when the record is
    /// absent, when it is already past its stored deadline (a lapsed record is
    /// never resurrected), or when `new_expires_at` would advance the stored
    /// deadline by less than `min_delta`. That last condition is what keeps a
    /// per-request call from becoming a per-request write: the number of
    /// writes over any window is bounded by the window divided by `min_delta`,
    /// whatever the request rate.
    ///
    /// The stored deadline never exceeds `absolute_cap`. A `new_expires_at`
    /// beyond it is clamped, and the clamped value is what `min_delta` is
    /// measured against — so a record already sitting at the cap stops
    /// generating writes rather than rewriting the same value forever.
    ///
    /// Implementations must apply all three conditions and the clamp
    /// atomically: concurrent callers on one key must not be able to observe
    /// or produce a torn deadline.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage fails.
    async fn touch_if_stale(
        &self,
        key: &SessionKey,
        new_expires_at: DateTime<Utc>,
        min_delta: Duration,
        absolute_cap: DateTime<Utc>,
    ) -> Result<SessionRenewal, SessionStoreError>;
}

/// Envelope-encryption body for session payloads.
///
/// Wraps/unwraps the per-session DEK via a [`KekSource`]; the crypto body
/// itself carries no KMS logic and no env/file logic — only the AEAD
/// envelope over whatever the `KekSource` returns.
#[async_trait]
pub trait EnvelopeCrypto: Send + Sync + 'static {
    /// Encrypt `plaintext` for `subject`, delegating KEK custody to
    /// `kek_source`. Returns the sealed ciphertext to hand to
    /// [`SessionStore::put`].
    ///
    /// # Arguments
    ///
    /// * `kek_source` - Key-encryption-key source for wrapping the DEK.
    /// * `subject` - Session identifier, used as AAD to bind the ciphertext.
    /// * `plaintext` - Data to encrypt.
    /// * `cookie_secret` - Per-session secret held by the client cookie,
    ///   mixed into the DEK so a store dump alone cannot decrypt.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeCryptoError::Seal`] if encryption fails,
    /// or [`EnvelopeCryptoError::Kek`] if the KEK source fails.
    async fn seal(
        &self,
        kek_source: &dyn KekSource,
        subject: &str,
        plaintext: &[u8],
        cookie_secret: &[u8],
    ) -> Result<Vec<u8>, EnvelopeCryptoError>;

    /// Decrypt `ciphertext` (as produced by [`Self::seal`]) for `subject`,
    /// delegating KEK custody to `kek_source`.
    ///
    /// # Arguments
    ///
    /// * `kek_source` - Key-encryption-key source for unwrapping the DEK.
    /// * `subject` - Session identifier, verified as AAD during decryption.
    /// * `ciphertext` - Sealed data to decrypt.
    /// * `cookie_secret` - Per-session secret held by the client cookie,
    ///   mixed into the DEK so a store dump alone cannot decrypt.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeCryptoError::Open`] if decryption fails,
    /// or [`EnvelopeCryptoError::Kek`] if the KEK source fails.
    async fn open(
        &self,
        kek_source: &dyn KekSource,
        subject: &str,
        ciphertext: &[u8],
        cookie_secret: &[u8],
    ) -> Result<Vec<u8>, EnvelopeCryptoError>;
}

/// Source of the server KEK used to wrap/unwrap the per-session DEK.
///
/// This is the ONLY tier seam between the community and enterprise builds:
/// the community build wires an operator-managed implementation, the
/// enterprise build wires a KMS-managed implementation. This trait delegates
/// ALL key custody — no wrap/unwrap-DEK, no KMS logic, and no env/file logic
/// exists anywhere outside implementations of this port.
#[async_trait]
pub trait KekSource: Send + Sync + 'static {
    /// Wrap `dek` (the per-session data-encryption key) for `subject`.
    ///
    /// # Errors
    ///
    /// Returns [`KekError::Wrap`] if wrapping the DEK fails.
    async fn wrap_dek(&self, subject: &str, dek: &[u8]) -> Result<Vec<u8>, KekError>;

    /// Unwrap a previously-wrapped DEK for `subject`.
    ///
    /// # Errors
    ///
    /// Returns [`KekError::Unwrap`] if unwrapping the DEK fails.
    async fn unwrap_dek(&self, subject: &str, wrapped_dek: &[u8]) -> Result<Vec<u8>, KekError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct DummySessionStore {
        storage: Mutex<HashMap<SessionKey, Vec<u8>>>,
    }

    #[async_trait]
    impl SessionStore for DummySessionStore {
        async fn put(
            &self,
            key: &SessionKey,
            value: Vec<u8>,
            _expires_at: DateTime<Utc>,
        ) -> Result<(), SessionStoreError> {
            self.storage.lock().unwrap().insert(key.clone(), value);
            Ok(())
        }

        async fn get(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError> {
            Ok(self.storage.lock().unwrap().get(key).cloned())
        }

        async fn delete(&self, key: &SessionKey) -> Result<(), SessionStoreError> {
            self.storage.lock().unwrap().remove(key);
            Ok(())
        }

        async fn consume(&self, key: &SessionKey) -> Result<Option<Vec<u8>>, SessionStoreError> {
            Ok(self.storage.lock().unwrap().remove(key))
        }

        // This double keeps no deadline, so it has no renewal to perform.
        // Reporting `NotRenewed` unconditionally is the only answer it can
        // give honestly — anything else would let a renewal test pass here
        // without a store that actually tracks deadlines.
        async fn touch_if_stale(
            &self,
            _key: &SessionKey,
            _new_expires_at: DateTime<Utc>,
            _min_delta: Duration,
            _absolute_cap: DateTime<Utc>,
        ) -> Result<SessionRenewal, SessionStoreError> {
            Ok(SessionRenewal::NotRenewed)
        }
    }

    struct DummyKekSource;

    #[async_trait]
    impl KekSource for DummyKekSource {
        async fn wrap_dek(&self, _subject: &str, dek: &[u8]) -> Result<Vec<u8>, KekError> {
            Ok(dek.to_vec())
        }

        async fn unwrap_dek(
            &self,
            _subject: &str,
            wrapped_dek: &[u8],
        ) -> Result<Vec<u8>, KekError> {
            Ok(wrapped_dek.to_vec())
        }
    }

    struct DummyEnvelopeCrypto;

    #[async_trait]
    impl EnvelopeCrypto for DummyEnvelopeCrypto {
        async fn seal(
            &self,
            kek_source: &dyn KekSource,
            subject: &str,
            plaintext: &[u8],
            _cookie_secret: &[u8],
        ) -> Result<Vec<u8>, EnvelopeCryptoError> {
            let wrapped = kek_source.wrap_dek(subject, plaintext).await?;
            Ok(wrapped)
        }

        async fn open(
            &self,
            kek_source: &dyn KekSource,
            subject: &str,
            ciphertext: &[u8],
            _cookie_secret: &[u8],
        ) -> Result<Vec<u8>, EnvelopeCryptoError> {
            let unwrapped = kek_source.unwrap_dek(subject, ciphertext).await?;
            Ok(unwrapped)
        }
    }

    #[tokio::test]
    async fn session_store_put_get_delete_round_trip() {
        let store = DummySessionStore {
            storage: Mutex::new(HashMap::new()),
        };

        let key = "test-key".to_string();
        let value = b"test-value".to_vec();
        let expires_at = Utc::now() + chrono::Duration::hours(1);

        store.put(&key, value.clone(), expires_at).await.unwrap();

        let retrieved = store.get(&key).await.unwrap();
        assert_eq!(retrieved, Some(value));

        store.delete(&key).await.unwrap();

        let after_delete = store.get(&key).await.unwrap();
        assert_eq!(after_delete, None);
    }

    #[tokio::test]
    async fn session_store_consume_removes_record() {
        let store = DummySessionStore {
            storage: Mutex::new(HashMap::new()),
        };

        let key = "one-shot".to_string();
        let value = b"code".to_vec();
        let expires_at = Utc::now() + chrono::Duration::hours(1);

        store.put(&key, value.clone(), expires_at).await.unwrap();

        let first = store.consume(&key).await.unwrap();
        assert_eq!(first, Some(value));

        let second = store.consume(&key).await.unwrap();
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn kek_source_wrap_unwrap_identity() {
        let kek_source = DummyKekSource;
        let subject = "test-subject";
        let dek = b"test-dek-data";

        let wrapped = kek_source.wrap_dek(subject, dek).await.unwrap();
        assert_eq!(wrapped, dek);

        let unwrapped = kek_source.unwrap_dek(subject, &wrapped).await.unwrap();
        assert_eq!(unwrapped, dek);
    }

    #[tokio::test]
    async fn envelope_crypto_seal_open_round_trip() {
        let crypto = DummyEnvelopeCrypto;
        let kek_source = DummyKekSource;
        let subject = "test-subject";
        let plaintext = b"test-plaintext";
        let cookie_secret = b"cookie-secret-value";

        let ciphertext = crypto
            .seal(&kek_source, subject, plaintext, cookie_secret)
            .await
            .unwrap();

        let decrypted = crypto
            .open(&kek_source, subject, &ciphertext, cookie_secret)
            .await
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn session_store_delete_idempotent() {
        let store = DummySessionStore {
            storage: Mutex::new(HashMap::new()),
        };

        let key = "nonexistent-key".to_string();

        store.delete(&key).await.unwrap();
    }

    #[tokio::test]
    async fn kek_source_is_object_safe() {
        let kek_source: &dyn KekSource = &DummyKekSource;
        let subject = "test";
        let dek = b"data";

        let wrapped = kek_source.wrap_dek(subject, dek).await.unwrap();
        let unwrapped = kek_source.unwrap_dek(subject, &wrapped).await.unwrap();

        assert_eq!(unwrapped, dek);
    }

    #[tokio::test]
    async fn envelope_crypto_with_dyn_kek_source() {
        let crypto = DummyEnvelopeCrypto;
        let kek_source: &dyn KekSource = &DummyKekSource;
        let subject = "test-subject";
        let plaintext = b"test-plaintext";
        let cookie_secret = b"cookie-secret-value";

        let ciphertext = crypto
            .seal(kek_source, subject, plaintext, cookie_secret)
            .await
            .unwrap();

        let decrypted = crypto
            .open(kek_source, subject, &ciphertext, cookie_secret)
            .await
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }
}
