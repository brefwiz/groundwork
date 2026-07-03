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
