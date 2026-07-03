// SPDX-License-Identifier: MIT

use aes_gcm::aead::{Aead, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce, aead::KeyInit};
use async_trait::async_trait;
use zeroize::Zeroizing;

use super::ports::{KekError, KekSource};

/// Community, operator-managed [`KekSource`].
///
/// The server KEK is supplied by the operator as raw key material held
/// OUTSIDE the session store — an environment variable, a mounted file, or a
/// platform secret. There is no KMS and no per-request network call: the KEK
/// lives in process memory for the lifetime of the source. This is the
/// community-tier counterpart to a KMS-backed source; the seam between the
/// two is [`KekSource`] alone.
///
/// The KEK wraps each per-session DEK with `AES-256-GCM`, framing the output
/// as `[12-byte nonce][ciphertext||tag]`.
pub struct EnvKekSource {
    key: Zeroizing<[u8; 32]>,
}

impl EnvKekSource {
    /// Construct from a 32-byte KEK the operator already holds in memory.
    #[must_use]
    pub fn from_bytes(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    /// Construct from operator-managed material read out of the process
    /// environment. `var` names an environment variable whose value is a
    /// 64-character lowercase-or-uppercase hex encoding of the 32-byte KEK.
    ///
    /// # Errors
    ///
    /// Returns [`KekError::Wrap`] if the variable is unset, is not valid hex,
    /// or does not decode to exactly 32 bytes.
    pub fn from_env(var: &str) -> Result<Self, KekError> {
        let raw = std::env::var(var)
            .map_err(|e| KekError::Wrap(format!("reading KEK env var '{var}': {e}")))?;
        Self::from_hex(raw.trim())
    }

    /// Construct from a hex-encoded 32-byte KEK. Useful when the operator
    /// mounts the key as a file and the caller reads it.
    ///
    /// # Errors
    ///
    /// Returns [`KekError::Wrap`] if `hex` is not valid hex or does not decode
    /// to exactly 32 bytes.
    pub fn from_hex(hex: &str) -> Result<Self, KekError> {
        let bytes =
            decode_hex(hex).map_err(|e| KekError::Wrap(format!("decoding KEK hex: {e}")))?;
        let key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| KekError::Wrap(format!("KEK must be 32 bytes, got {}", bytes.len())))?;
        Ok(Self::from_bytes(key))
    }
}

#[async_trait]
impl KekSource for EnvKekSource {
    async fn wrap_dek(&self, _subject: &str, dek: &[u8]) -> Result<Vec<u8>, KekError> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| KekError::Wrap(format!("cipher init: {e}")))?;

        let encrypted = cipher
            .encrypt(&nonce, dek)
            .map_err(|e| KekError::Wrap(format!("encryption failed: {e}")))?;

        let mut wrapped = Vec::with_capacity(12 + encrypted.len());
        wrapped.extend_from_slice(&nonce_bytes);
        wrapped.extend_from_slice(&encrypted);
        Ok(wrapped)
    }

    async fn unwrap_dek(&self, _subject: &str, wrapped_dek: &[u8]) -> Result<Vec<u8>, KekError> {
        if wrapped_dek.len() < 12 {
            return Err(KekError::Unwrap("wrapped DEK too short".into()));
        }
        let (nonce_bytes, encrypted) = wrapped_dek.split_at(12);

        let mut nonce_array = [0u8; 12];
        nonce_array.copy_from_slice(nonce_bytes);
        let nonce = Nonce::from(nonce_array);

        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| KekError::Unwrap(format!("cipher init: {e}")))?;

        cipher
            .decrypt(&nonce, encrypted)
            .map_err(|e| KekError::Unwrap(format!("decryption failed: {e}")))
    }
}

/// Decode a hex string into bytes without pulling an extra dependency.
fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd number of hex digits".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn env_kek_wrap_unwrap_round_trip() {
        let source = EnvKekSource::from_bytes([7u8; 32]);
        let dek = b"a-32-byte-data-encryption-key!!!";

        let wrapped = source.wrap_dek("subject", dek).await.unwrap();
        assert_ne!(wrapped.as_slice(), dek);

        let unwrapped = source.unwrap_dek("subject", &wrapped).await.unwrap();
        assert_eq!(unwrapped.as_slice(), dek);
    }

    #[tokio::test]
    async fn env_kek_tampered_wrapped_dek_fails() {
        let source = EnvKekSource::from_bytes([9u8; 32]);
        let mut wrapped = source.wrap_dek("subject", b"secret-dek").await.unwrap();
        let last = wrapped.len() - 1;
        wrapped[last] ^= 0xFF;
        assert!(source.unwrap_dek("subject", &wrapped).await.is_err());
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(EnvKekSource::from_hex("00ff").is_err());
    }

    #[test]
    fn from_hex_accepts_32_bytes() {
        let hex = "00".repeat(32);
        assert!(EnvKekSource::from_hex(&hex).is_ok());
    }
}
