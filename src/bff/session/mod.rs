// SPDX-License-Identifier: MIT

//! BFF session TCB module — ports only.

pub mod ports;

#[cfg(all(feature = "bff", feature = "database"))]
pub mod postgres_store;

pub use ports::{
    EnvelopeCrypto, EnvelopeCryptoError, KekError, KekSource, SessionKey, SessionStore,
    SessionStoreError,
};

#[cfg(all(feature = "bff", feature = "database"))]
pub use postgres_store::PostgresSessionStore;
