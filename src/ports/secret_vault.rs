//! Secret vault port — envelope-encrypted secret storage.
//!
//! This trait uses plain `Vec<u8>` for secret bytes and `String` for identifiers,
//! so consumers have no dependency on internal crates.

use std::future::Future;
use std::pin::Pin;

/// Error type for vault operations.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The secret was not found.
    #[error("secret not found")]
    NotFound,
    /// The vault backend returned an error.
    #[error("vault backend error: {0}")]
    Backend(String),
    /// Access denied.
    #[error("forbidden")]
    Forbidden,
}

/// Port: envelope-encrypted secret vault.
///
/// Consumers implement this trait against their preferred KMS backend.
/// The trait uses plain Rust types — no dependency on internal crates.
pub trait SecretVault: Send + Sync + 'static {
    /// Envelope-encrypt `plaintext` and persist it under `(namespace, id)`.
    fn put(
        &self,
        namespace: &str,
        id: &str,
        plaintext: Vec<u8>,
        principal: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VaultError>> + Send>>;

    /// Retrieve and decrypt the secret under `(namespace, id)`.
    fn get(
        &self,
        namespace: &str,
        id: &str,
        principal: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, VaultError>> + Send>>;

    /// Delete the secret and its wrapped DEK.
    fn delete(
        &self,
        namespace: &str,
        id: &str,
        principal: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VaultError>> + Send>>;

    /// Re-wrap the DEK for `(namespace, id)` under the current root key version.
    fn rewrap(
        &self,
        namespace: &str,
        id: &str,
        principal: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VaultError>> + Send>>;
}
