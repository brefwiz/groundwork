// SPDX-License-Identifier: MIT

use async_trait::async_trait;
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

/// Server-side store for opaque, cookie-keyed BFF session records.
///
/// Implementations own persistence only. The stored `value` is always the
/// already envelope-encrypted blob produced by [`EnvelopeCrypto::seal`] —
/// the store itself is zero-knowledge of plaintext.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Persist `value` under `key`, replacing any existing record.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError::Backend`] if the underlying storage
    /// fails, or other variants for specific failure modes.
    async fn put(&self, key: &SessionKey, value: Vec<u8>) -> Result<(), SessionStoreError>;

    /// Fetch the record stored under `key`, if any.
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
    /// * `kek_source` - Key encryption key source for wrapping the DEK
    /// * `subject` - Session identifier, used as AAD to bind encryption
    /// * `plaintext` - Data to encrypt
    /// * `cookie_secret` - Per-session secret from httpOnly cookie, mixed with unwrapped DEK
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
    /// * `kek_source` - Key encryption key source for unwrapping the DEK
    /// * `subject` - Session identifier, verified as AAD during decryption
    /// * `ciphertext` - Sealed data to decrypt
    /// * `cookie_secret` - Per-session secret from httpOnly cookie, mixed with unwrapped DEK
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
/// This is the ONLY tier seam between community and enterprise: community
/// wires an operator-managed implementation, enterprise wires a
/// KMS-managed implementation. This trait delegates ALL key custody —
/// no wrap/unwrap-DEK, no KMS logic, and no env/file logic exists anywhere
/// outside implementations of this port.
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
        async fn put(&self, key: &SessionKey, value: Vec<u8>) -> Result<(), SessionStoreError> {
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
    }

    struct DummyKekSource;

    #[async_trait]
    impl KekSource for DummyKekSource {
        async fn wrap_dek(&self, _subject: &str, dek: &[u8]) -> Result<Vec<u8>, KekError> {
            // Identity wrap: just return the DEK as-is for testing purposes
            Ok(dek.to_vec())
        }

        async fn unwrap_dek(
            &self,
            _subject: &str,
            wrapped_dek: &[u8],
        ) -> Result<Vec<u8>, KekError> {
            // Identity unwrap: just return the wrapped DEK as-is for testing purposes
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
            // Simple implementation: wrap the plaintext via KEK source
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
            // Simple implementation: unwrap the ciphertext via KEK source
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

        // Put
        store.put(&key, value.clone()).await.unwrap();

        // Get
        let retrieved = store.get(&key).await.unwrap();
        assert_eq!(retrieved, Some(value));

        // Delete
        store.delete(&key).await.unwrap();

        // Verify deletion
        let after_delete = store.get(&key).await.unwrap();
        assert_eq!(after_delete, None);
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

        // Deleting a non-existent key should not error
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
