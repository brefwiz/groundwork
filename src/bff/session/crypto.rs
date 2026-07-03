// SPDX-License-Identifier: MIT

use aes_gcm::aead::{Aead, OsRng, Payload, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce, aead::KeyInit};
use async_trait::async_trait;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

use super::ports::{EnvelopeCrypto, EnvelopeCryptoError, KekError, KekSource};

/// AEAD envelope-encryption implementation using `AES-256-GCM`.
///
/// Generates a random per-session DEK, AEAD-encrypts plaintext under that DEK,
/// and wraps the DEK via an injected [`KekSource`]. The DEK is KDF-mixed with
/// a per-session cookie secret to prevent store-dump-alone decryption.
/// No KMS, env, or file logic anywhere in this module — all key custody is
/// delegated to the injected source.
pub struct AeadEnvelopeCrypto;

#[async_trait]
impl EnvelopeCrypto for AeadEnvelopeCrypto {
    async fn seal(
        &self,
        kek_source: &dyn KekSource,
        subject: &str,
        plaintext: &[u8],
        cookie_secret: &[u8],
    ) -> Result<Vec<u8>, EnvelopeCryptoError> {
        // Generate a random DEK (32 bytes = AES-256 key size).
        let mut dek = [0u8; 32];
        OsRng.fill_bytes(&mut dek);

        // KDF-mix the DEK with the cookie secret to derive the final encryption key.
        let mut final_dek = [0u8; 32];
        let hkdf = Hkdf::<Sha256>::new(Some(cookie_secret), &dek);
        hkdf.expand(subject.as_bytes(), &mut final_dek)
            .map_err(|e| EnvelopeCryptoError::Seal(format!("KDF expansion failed: {e}")))?;

        // Generate a random nonce (12 bytes for GCM).
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        // AEAD-encrypt plaintext under the derived DEK, AAD-bound to subject.
        let cipher = Aes256Gcm::new_from_slice(&final_dek)
            .map_err(|e| EnvelopeCryptoError::Seal(format!("cipher init: {e}")))?;

        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: subject.as_bytes(),
                },
            )
            .map_err(|e| EnvelopeCryptoError::Seal(format!("encryption failed: {e}")))?;

        // Wrap the raw DEK (not the final derived key) via the KEK source.
        let wrapped_dek = kek_source.wrap_dek(subject, &dek).await?;

        // Zero the raw DEKs.
        dek.zeroize();
        final_dek.zeroize();

        // Serialize framing: [4-byte LE wrapped_dek_len][wrapped_dek][12-byte nonce][ciphertext].
        let wrapped_dek_len_u32 = u32::try_from(wrapped_dek.len())
            .map_err(|_| EnvelopeCryptoError::Seal("wrapped DEK too large".into()))?;

        let mut envelope = Vec::new();
        envelope.extend_from_slice(&wrapped_dek_len_u32.to_le_bytes());
        envelope.extend_from_slice(&wrapped_dek);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);

        Ok(envelope)
    }

    async fn open(
        &self,
        kek_source: &dyn KekSource,
        subject: &str,
        ciphertext: &[u8],
        cookie_secret: &[u8],
    ) -> Result<Vec<u8>, EnvelopeCryptoError> {
        // Parse framing: [4-byte LE wrapped_dek_len][wrapped_dek][12-byte nonce][ciphertext].
        if ciphertext.len() < 4 + 12 {
            return Err(EnvelopeCryptoError::Open(
                "envelope too short to parse framing".into(),
            ));
        }

        // Read wrapped_dek length (4 bytes LE).
        let wrapped_dek_len =
            u32::from_le_bytes([ciphertext[0], ciphertext[1], ciphertext[2], ciphertext[3]])
                as usize;

        // Ensure we have enough bytes for wrapped_dek, nonce, and ciphertext.
        let required_len = 4 + wrapped_dek_len + 12;
        if ciphertext.len() < required_len {
            return Err(EnvelopeCryptoError::Open(
                "envelope too short for declared wrapped_dek_len".into(),
            ));
        }

        let wrapped_dek = &ciphertext[4..4 + wrapped_dek_len];
        let nonce_bytes = &ciphertext[4 + wrapped_dek_len..4 + wrapped_dek_len + 12];
        let encrypted_payload = &ciphertext[4 + wrapped_dek_len + 12..];

        // Unwrap the DEK via the KEK source.
        let mut unwrapped_dek = kek_source.unwrap_dek(subject, wrapped_dek).await?;

        // KDF-mix the unwrapped DEK with the cookie secret to derive the final DEK.
        let mut dek = [0u8; 32];
        let hkdf = Hkdf::<Sha256>::new(Some(cookie_secret), &unwrapped_dek);
        hkdf.expand(subject.as_bytes(), &mut dek)
            .map_err(|e| EnvelopeCryptoError::Open(format!("KDF expansion failed: {e}")))?;

        // Zero the unwrapped DEK.
        unwrapped_dek.zeroize();

        // Decrypt.
        let mut nonce_array = [0u8; 12];
        nonce_array.copy_from_slice(nonce_bytes);
        let nonce = Nonce::from(nonce_array);
        let cipher = Aes256Gcm::new_from_slice(&dek)
            .map_err(|e| EnvelopeCryptoError::Open(format!("cipher init: {e}")))?;

        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: encrypted_payload,
                    aad: subject.as_bytes(),
                },
            )
            .map_err(|e| EnvelopeCryptoError::Open(format!("decryption failed: {e}")))?;

        // Zero the raw DEK.
        dek.zeroize();

        Ok(plaintext)
    }
}

/// In-process test KEK source for testing only.
///
/// This struct is gated behind `test` or the `test-crypto` feature and
/// is NOT reachable in a production build. It performs a trivial
/// deterministic AEAD wrap/unwrap for testing purposes.
#[cfg(any(test, feature = "test-crypto"))]
pub struct InProcessTestKekSource {
    key: [u8; 32],
}

#[cfg(any(test, feature = "test-crypto"))]
impl InProcessTestKekSource {
    /// Create a new in-process test KEK source with a fixed key.
    ///
    /// # Panics
    ///
    /// Panics if the key is not exactly 32 bytes.
    #[must_use]
    pub fn new(key: &[u8; 32]) -> Self {
        Self { key: *key }
    }
}

#[cfg(any(test, feature = "test-crypto"))]
#[async_trait]
impl KekSource for InProcessTestKekSource {
    async fn wrap_dek(&self, _subject: &str, dek: &[u8]) -> Result<Vec<u8>, KekError> {
        // Generate a nonce for the wrap operation.
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        // AEAD-encrypt the DEK under the test key.
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| KekError::Wrap(format!("cipher init: {e}")))?;

        let encrypted = cipher
            .encrypt(&nonce, dek)
            .map_err(|e| KekError::Wrap(format!("encryption failed: {e}")))?;

        // Frame as [12-byte nonce][encrypted_dek].
        let mut wrapped = Vec::new();
        wrapped.extend_from_slice(&nonce_bytes);
        wrapped.extend_from_slice(&encrypted);

        Ok(wrapped)
    }

    async fn unwrap_dek(&self, _subject: &str, wrapped_dek: &[u8]) -> Result<Vec<u8>, KekError> {
        // Parse framing: [12-byte nonce][encrypted_dek].
        if wrapped_dek.len() < 12 {
            return Err(KekError::Unwrap("wrapped DEK too short".into()));
        }

        let nonce_bytes = &wrapped_dek[..12];
        let encrypted = &wrapped_dek[12..];

        let mut nonce_array = [0u8; 12];
        nonce_array.copy_from_slice(nonce_bytes);
        let nonce = Nonce::from(nonce_array);

        // AEAD-decrypt the DEK under the test key.
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| KekError::Unwrap(format!("cipher init: {e}")))?;

        cipher
            .decrypt(&nonce, encrypted)
            .map_err(|e| KekError::Unwrap(format!("decryption failed: {e}")))
    }
}
